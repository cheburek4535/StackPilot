# Settings — creative ideas (awaiting approval)

Each idea below is self-contained, honest, and implementable without a
token sink. Marked with estimated effort (S = small, M = medium) and
whether it needs backend changes.

## A. System file/folder pickers for path fields (S, frontend + existing plugin)
Buttons "Browse…" next to VS Code / browser / terminal paths that open the
native file dialog (`tauri-plugin-dialog` is already installed). No more
copy-pasting paths for beginners.

## B. Live path existence check (S, new tiny backend command)
`settings_check_path(path)` → bool. A green dot next to a path that exists,
red dot + hint when it does not. Catches typos in `code`, `git`, etc.
before a launch profile silently fails.

## C. "Open app data folder" button (S, new tiny backend command)
Opens the folder where StackPilot stores `settings.json` and profiles
(`tauri-plugin-opener` is already installed). Power users can back up or
inspect everything in two clicks.

## D. "About" block + theme preview cards (S, frontend only)
A small card with app version, environment info and a visual preview of
Light/Dark theme + current accent color. Builds trust and looks serious
for newcomers.

## E. Export / import settings as JSON (M, backend + frontend)
"Export" saves the current settings to a user-chosen `.json` file via the
native save dialog; "Import" reads one back and applies it. Useful for
backup and moving settings between machines. Data is already JSON on disk,
so this is mostly plumbing.

## F. Soft validation for email / URL / key fields (S, frontend only)
Inline hints under Personal and AI fields: email format, `https://` URLs,
API key length. Never blocks saving — just warns, which is exactly right
for a dead-but-saved AI config.