# GPUI 实体事件机制

> **目的**：说明 GPUI **实体级**事件（`EventEmitter` / `emit` / `subscribe`），不是全局 EventBus。  
> **约定**：中文；对应 gpui **0.2.x**（以 crate 源码为准）。

## 一句话

某个 **Entity** 通过 `cx.emit(事件)` 广播；其他方事先用 `subscribe` / `subscribe_in` 登记回调；事件作为 `Effect::Emit` 进入 pending 队列，在本轮 `flush_effects` 时投递给匹配的订阅者。

```text
Emitter Entity                         Subscriber
┌─────────────────┐                    ┌──────────────────┐
│  impl           │  emit(Evt)         │  Context / App / │
│  EventEmitter<E>│ ─────────────────► │  Window          │
└─────────────────┘   Effect::Emit     │  subscribe(...)  │
                                       └──────────────────┘
```

## 和「全局事件总线」的区别

| | GPUI 实体事件 | 典型 EventBus |
|--|---------------|---------------|
| 范围 | **某一 Entity 实例** 发出 | 任意模块任意发 |
| 类型 | `EventEmitter<Evt>` 编译期绑死「谁能发哪种 Evt」 | 常靠字符串 / `Any` |
| 路由 | 按 `EntityId` + `TypeId::of::<Evt>()` 匹配监听器 | 常靠 topic 名 |
| 生命周期 | 返回 `Subscription`；drop 即取消 | 易泄漏、难排查 |

## 三件套

### 1. 声明事件类型 + `EventEmitter`

`EventEmitter` 是空 trait，只做 **类型绑定**（编译期：这个 Entity 允许 emit 这种事件）：

```rust
pub trait EventEmitter<E: Any>: 'static {}
```

```rust
#[derive(Clone, Debug)]
pub enum ChildEvent {
    Clicked,
    Closed,
}

struct Child;

impl EventEmitter<ChildEvent> for Child {}
```

同一 Entity 可以对 **多种** 事件类型分别实现：

```rust
impl EventEmitter<ChildEvent> for Child {}
impl EventEmitter<OtherEvent> for Child {}
```

### 2. 发出：`Context::emit`

在 **发射方自己的** `Context<'_, Self>` 里调用：

```rust
cx.emit(ChildEvent::Clicked);
```

约束：`Self: EventEmitter<Evt>`，且 `Evt: 'static`。

实现上不会立刻同步跑完所有订阅者，而是：

```rust
pending_effects.push_back(Effect::Emit {
    emitter: entity_id,
    event_type: TypeId::of::<Evt>(),
    event: Box::new(event),
});
```

在 `App::update` 结束时的 `flush_effects` 里再 `apply_emit_effect`：按发射方 `EntityId` 取出监听器，仅当监听器登记的 `TypeId` 与本次事件一致时调用回调。

因此：

- emit 返回后，订阅方 **未必** 已跑完；
- 不要假设「emit 之后订阅方状态已更新」。

### 3. 订阅

按是否需要 `Window`、以及在谁的 Context 上订，常用几条 API：

| API | 大致回调 | 何时用 |
|-----|----------|--------|
| `Context::subscribe(entity, …)` | `\|this, Entity\<Emitter\>, &Evt, &mut Context\<Self\>\|` | 订阅方是 Entity，不需要 `Window` |
| `Context::subscribe_in(entity, window, …)` | 多一个 `&mut Window` | 回调里要焦点、窗口级 UI |
| `Context::subscribe_self(…)` | `\|this, &Evt, &mut Context\<Self\>\|` | 订自己发出的事件 |
| `App::subscribe(entity, …)` | `\|Entity\<Emitter\>, &Evt, &mut App\|` | 在 App 层订阅（无订阅方 Entity） |
| `Window::subscribe(entity, cx, …)` | `\|Entity\<Emitter\>, &Evt, &mut Window, &mut App\|` | 在窗口上下文里订阅 |

`Context::subscribe` 示例：

```rust
let sub = cx.subscribe(&child, |this, child, event: &ChildEvent, cx| {
    match event {
        ChildEvent::Clicked => { /* ... */ }
        ChildEvent::Closed => { /* ... */ }
    }
});
```

`subscribe_in` 示例（需要窗口）：

```rust
let sub = cx.subscribe_in(&child, window, |this, child, event, window, cx| {
    // 可使用 window，例如改焦点
});
```

监听器按 **发射方 EntityId + 事件 TypeId** 登记；订阅方通常以 **weak** 持有，升级失败则回调不再执行（并可能被从列表中剔除）。

## `Subscription` 生命周期

```rust
#[must_use]
pub struct Subscription { /* ... */ }
```

- **Drop 即取消**：`Subscription` 被 drop 时调用内部 unsubscribe。
- **必须挂住**：局部 `let _ = cx.subscribe(...)` 会立刻取消；应存进字段（如 `_subscriptions: Vec<Subscription>`）或显式 `detach()`。
- **`detach()`**：从句柄上拆开，回调会一直有效，直到相关 Entity 释放。
- **`join(a, b)`**：两个订阅合成一个句柄，一起 drop / detach。

## 最小心智模型

```rust
use gpui::{Context, Entity, EventEmitter, Subscription};

struct Child;
enum ChildEvent { Clicked }
impl EventEmitter<ChildEvent> for Child {}

impl Child {
    fn on_click(&mut self, cx: &mut Context<Self>) {
        cx.emit(ChildEvent::Clicked);
    }
}

struct Parent {
    child: Entity<Child>,
    _sub: Subscription,
}

impl Parent {
    fn new(cx: &mut Context<Self>) -> Self {
        let child = cx.new(|_| Child);
        let _sub = cx.subscribe(&child, |_this, _child, ev: &ChildEvent, _cx| {
            match ev {
                ChildEvent::Clicked => { /* 父级处理 */ }
            }
        });
        Self { child, _sub }
    }
}
```

## 分发时序（简图）

```text
cx.emit(evt)
    │
    ▼
pending_effects ← Effect::Emit { emitter, event_type, event }
    │
    │  （本轮 App::update 末尾）
    ▼
flush_effects
    │
    ▼
apply_emit_effect：对 emitter 的 listeners
    若 TypeId 匹配 → 调用 handler(&event, app)
```

`flush_effects` 会循环处理：回调里再 `emit` / `notify` / `defer` 产生的新 Effect，会继续排到队列里直到清空。

## 常见注意点

1. **只订「这一份」Entity**：订阅的是具体 `Entity<T>` 句柄，不是「所有 `T`」。
2. **类型必须对上**：`EventEmitter<Evt>` + `TypeId::of::<Evt>()`；订错事件类型不会收到。
3. **异步投递**：不要把 emit 当同步 RPC。
4. **挂住 Subscription**：否则看起来「订了但从不触发」。
5. **发射方 / 订阅方释放**：任一侧释放后，依赖 weak 的回调会停；长期订阅应随拥有方一起存放。

## 延伸阅读（gpui 源码）

- `EventEmitter`：`gpui.rs`
- `Context::emit` / `subscribe` / `subscribe_in` / `subscribe_self`：`app/context.rs`
- `App::subscribe`、`flush_effects` / `apply_emit_effect`：`app.rs`
- `Window::subscribe`：`window.rs`
- `Subscription`：`subscription.rs`
