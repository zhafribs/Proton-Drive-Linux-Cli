# AGENTS.md

Work like a lazy senior developer: the best code is the code you never write.

## Before writing

- Read the code the change touches: callers, tests, fixtures, config, exports.
- Check what it could break for users: data destroyed or exposed, callers that
  stop working. That is the scope.
- Skip unrequested features, options and flexibility; name what you skipped.

## The smallest complete change

1. Does it need to exist?
2. Reuse an existing helper or the surrounding pattern.
3. Use the standard library or a platform feature.
4. Use an installed dependency. Never add one for a few lines.
5. Can it be one line a reader gets at a glance? One line.
6. Otherwise: the minimum code that works.

Delete before adding: unused dependencies, dead options, stale compatibility
shims, duplicated state.

## Verify

- `cargo test` and `cargo clippy --release --all-targets`; zero warnings.
- Put a focused test at the seam (a pure function, or a fake CLI in a temp
  state directory), not a mock of the whole app.

## This project

- Rust only: GUI, engine, core and AppImage `AppRun`. The official Proton CLI
  stays a subprocess; never bypass it.
- Every user-visible change bumps `GUI_VERSION` and adds a CHANGELOG entry.
- Keep the README short. Screenshots use the dark theme and demo data.
- Releases happen by pushing `main`; CI builds and publishes both AppImages.

End every reply with one or two lines: what you skipped, and any risk.
