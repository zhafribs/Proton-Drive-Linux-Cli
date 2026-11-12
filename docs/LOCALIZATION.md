# Language support

The UI language is stored as `language=en|ms` in the preferences file.

## Bahasa Melayu (beta)

Covered:

- first-run setup wizard
- sidebar navigation and account menu
- sync status titles, subtitles and the status card
- Home: actions, metrics, cloud storage, sync details, activity preview

Not covered yet:

- My files, Photos, Activity, Settings, About and Recovery bodies
- dialogs, validation messages and notifications
- dates and relative times

The Appearance panel says this explicitly. A partially translated page is
worse than an honest English fallback, so untranslated strings stay English
rather than being machine-translated.

## Adding a language

`gui/src/i18n.rs` holds the runtime table. `tr("English", "translation")` is
used at the call site; untranslated code is obvious because it has no `tr`
wrapper. Add the enum variant, the preference code, and a selector entry.
