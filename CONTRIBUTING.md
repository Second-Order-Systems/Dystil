# Contributing to Dystil

Contributions are welcome. Start with [`AGENTS.md`](AGENTS.md) — it is the fastest
orientation for both people and coding agents, and it covers build commands, the
repository layout, and the conventions that will otherwise trip you up.

## Sign your commits (DCO)

This project uses the [Developer Certificate of Origin](https://developercertificate.org/).
There is no CLA and nothing to sign up for — you keep the copyright in your work.
You just certify that you have the right to submit it, by adding a `Signed-off-by`
line to each commit:

```bash
git commit -s -m "your message"
```

That appends:

```
Signed-off-by: Your Name <your.email@example.com>
```

Use your real name and an email you can be reached at. If you forget, `git commit
--amend -s` fixes the last commit and `git rebase --signoff main` fixes a branch.

## Before you open a pull request

```bash
cd apps/dystil
bun run test          # vitest + bun test
bun run typecheck
bun run bindings:check

cargo fmt
cargo clippy -p dystil-<crate> --all-targets -- -W clippy::all
cargo test  -p dystil-<crate>
```

Install the hooks once with `bunx lefthook install`. They run `cargo fmt --check`,
scoped clippy, and the documentation citation check on staged files.

## Things that will save you a round trip

- **Run the app with `bunx tauri dev`, not `cargo run`.** `cargo run` skips Tauri's
  `beforeDevCommand`, so the frontend never starts.
- **Rust → TypeScript bindings are generated.** After changing a Tauri command
  signature or a shared type, run `bun run bindings:generate`. CI fails on stale
  bindings.
- **`dystil-redact` is text-only.** Images are never inspected. Keep it that way.
- **`dystil-work-index` is deliberately dumb.** It records observable continuity, not
  intent or causality. Inference belongs in `dystil-insights`.
- **Never implement from `public_docs/`.** It is positioning and is allowed to run
  ahead of the code. `agent_docs/` is the verified reference and cites the code.
- **Privacy is structural.** If a change causes more data to leave the device, it
  needs an explicit opt-in and a line in `agent_docs/PRIVACY_AND_TELEMETRY.md`.

## Licence

By contributing you agree that your contribution is licensed under the
[Apache License 2.0](LICENSE), the same terms as the rest of the desktop
application.

## Reporting security issues

Please report security issues privately to <udit@2os.ai> rather than opening a
public issue.
