# SSH ProxyJump (one hop)

Related: [ARCHITECTURE.md](./ARCHITECTURE.md), [DECISIONS.md](./DECISIONS.md), [BACKLOG.md](./BACKLOG.md) (P1).

## Behavior

Loom supports **one** ProxyJump hop, equivalent to:

```text
ssh -J jumpuser@jumphost:jumpport targetuser@targethost:port
```

This is **not** Local port forwarding (`-L`). The jump host opens a `direct-tcpip` channel to the target; the target SSH session (shell, SFTP, Local forwards) rides that tunnel. The jump `Handle` stays alive for the whole target session.

## Profile model

`ProfileKind::Ssh` may include optional `jump: SshJumpHost { host, port, user, auth }`. Older workspace files omit it (`None`).

Passwords:

| Role | Keyring user (service `loom`) |
|------|-------------------------------|
| Target | `ssh/{profile_id}` |
| Jump | `ssh-jump/{profile_id}` (legacy `ssh/{id}/jump` is migrated on read) |

Jump uses a **separate user prefix** on purpose: on Windows the Cred Manager target is `{user}.loom`, and a nested `ssh/{id}/jump` could interfere with `ssh/{id}`. |

Both can use password (Remember → keyring) or private key. Connect prompts collect jump first, then target, when credentials are missing. Checking **Remember** on a connect prompt also sets the profile’s `remember: true` so a later profile Save with a blank password does not wipe the keyring entry.

## UI

SSH profile form → **Jump host (ProxyJump)** checkbox → host / port / user + password or key + Remember.

Jump vs direct SSH cues:

| Surface | Jump |
|---------|------|
| Status bar | Target reads `user@host:port via jumpuser@jumphost:jport`; remote icon uses jump accent |
| Tab title | Muted `via` next to the name |
| Sidebar | Muted `via` badge + jump-colored remote icon |
| Connecting | Status text includes the same `via …` endpoint |

Profile summary (Context / picker): `ssh · user@host:port via jumpuser@jumphost:jport`.

## Limits (MVP)

- One hop only (no jump chains).
- No jump `session_password` on panes — reconnect without a stored jump password re-prompts.
- Agent / passphrase UI still open (see BACKLOG P1).
