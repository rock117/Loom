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

| Role | Keyring service entry |
|------|------------------------|
| Target | `ssh/{profile_id}` |
| Jump | `ssh/{profile_id}/jump` |

Both can use password (Remember → keyring) or private key. Connect prompts collect jump first, then target, when credentials are missing.

## UI

SSH profile form → **Jump host (ProxyJump)** checkbox → host / port / user + password or key + Remember. Sidebar summary: `ssh · user@host:port via jumpuser@jumphost:jport`.

## Limits (MVP)

- One hop only (no jump chains).
- No jump `session_password` on panes — reconnect without a stored jump password re-prompts.
- Agent / passphrase UI still open (see BACKLOG P1).
