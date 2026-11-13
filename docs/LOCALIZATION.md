# Language support

The UI language is stored as `language=en|ms` in the preferences file and can
be changed during first-run setup or in Settings → Appearance.

## Bahasa Melayu

Covered:

- all pages: Home, My files, Photos, Activity, Settings, About, Recovery
- first-run wizard, all dialogs, menus, tray entries and notifications
- sync status, empty states, errors shown by the app, update progress
- counts, relative times and journal verbs (`4 item`, `12 minit lalu`,
  `Dimuat naik · ...`)

Not translated:

- file and folder names from the cloud or disk
- raw CLI/API error text, which is quoted verbatim so it can be searched
- the language names themselves in the selector

The catalog lives in `gui/src/i18n.rs`. `tr()` is used where the string is
known at the call site; `t()`/`rt()` translate static and formatted display
text. Unknown strings fall back to English rather than showing a blank.

## Adding a language

1. Add a variant and code in `gui/src/i18n.rs`.
2. Add the selector entry.
3. Extend `t()` and the `tr()` call sites.
4. Run `scripts/ui-check.sh` and inspect `docs/audit/m11/` for clipped text.
