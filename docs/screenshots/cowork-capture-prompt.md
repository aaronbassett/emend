# Cowork capture prompt — Emend README screenshots

Paste everything in the **fenced block below** into Claude Cowork (with computer-use /
desktop control enabled). It will drive the already-running **Emend** app, reach each
feature state, and save 9 PNGs into `docs/screenshots/` with the exact filenames the
README expects.

**Before you start (already done for you):**
- Emend is built and running.
- A demo vault is seeded at `~/EmendDemo`.
- `paneful` + `screencapture` work and the terminal has Screen Recording permission.

If Emend ever isn't open, launch it with:
`open ~/Library/Developer/Xcode/DerivedData/Emend-*/Build/Products/Debug/Emend.app`

---

```text
You are controlling my Mac to capture product screenshots of a native macOS Markdown
editor called **Emend** for its README. Emend is already running, and a demo vault is
seeded at ~/EmendDemo. Work carefully and visually verify each shot before moving on.

## How to capture (do this for every shot)

The cleanest capture is a single-window PNG (transparent background + drop shadow).
In a Terminal, define this helper once:

    mkdir -p "$HOME/Projects/aaronbassett/emend/docs/screenshots"
    cap () { WID=$(paneful --app Emend --json | jq -r '[.[]|select(.is_onscreen)][0].window_id'); screencapture -l "$WID" "$HOME/Projects/aaronbassett/emend/docs/screenshots/$1.png"; }

Then, once the app is in the right state, run `cap <name>` (e.g. `cap hero`).
`paneful` lists Emend's windows front-to-back, so the helper grabs the FRONTMOST one —
which is the main window for most shots, and the settings sheet for the sheet shots.

If you cannot type into a Terminal, instead use the macOS window screenshot
(Shift-Cmd-4, then press Space, then click the Emend window) and save the file to
`~/Projects/aaronbassett/emend/docs/screenshots/<name>.png`.

After each capture, take a normal screenshot to confirm the PNG looks right (no stray
menu open, no mouse cursor mid-window, the content is what's described). Re-take if not.

## One-time setup

1. Activate Emend (click its window or its Dock icon).
2. Resize the main Emend window to a generous size — about 1440 x 900 — and position it
   fully on screen (not under the menu bar/notch). You can drag the bottom-right corner,
   or run in Terminal:
     osascript -e 'tell application "System Events" to tell process "Emend" to set position of front window to {140, 90}'
     osascript -e 'tell application "System Events" to tell process "Emend" to set size of front window to {1440, 900}'
   (If macOS asks for Accessibility permission for the Terminal, grant it, or just
   resize by dragging.)
3. Add the demo folder: in Emend's toolbar click **Add Location** (the folder icon with a
   small “+”). In the Open panel, press Cmd-Shift-G, type `~/EmendDemo`, press Return,
   then click **Open**. The sidebar should now show: Welcome, Diagrams & Math, Reading
   List, and folders Projects/ and Daily/.
4. In the sidebar, click the disclosure triangles to expand **Projects** and **Daily** so
   the tree looks full.
5. Make sure the app is in **Dark** appearance (it looks best for these shots). If it's
   light, switch macOS to Dark in System Settings > Appearance, or leave it — just be
   consistent across all shots.

Toolbar buttons you'll use (hover to confirm the tooltip):
- **Add Location** — folder icon with “+”.
- **Toggle Preview** — icon of a rectangle split into two panes (tooltip “Show or hide the live preview”).
- **Typography** — the “AA” button (tooltip “Font, size, and spacing”).
- **AI** — the sparkles icon with a dropdown chevron (tooltip “BYOM AI summary”).

## The 9 shots

### 1. editor.png  (the editor, no preview)
- Make sure the preview is OFF (if a preview pane is showing on the right of the editor,
  click **Toggle Preview** to hide it).
- Open the **Welcome** note from the sidebar.
- The editor should show the Welcome note with its headings, the task list, the Swift code
  block, and the table. Run: `cap editor`

### 2. info-sidebar.png  (live outline + stats)
- Keep preview OFF. Open the **Diagrams & Math** note from the sidebar.
- The far-right Info pane should show an Outline (the note's headings) and stats
  (words / characters / reading time / tasks). Run: `cap info-sidebar`

### 3. workspace.png  (sidebar + tabs)
- Keep preview OFF. Open three notes so there are tabs across the top: **Welcome**,
  **Projects/Q3 Roadmap**, and **Diagrams & Math**. Leave Q3 Roadmap as the active tab.
- The sidebar should show the full expanded tree (Projects/ and Daily/ open). Run: `cap workspace`

### 4. hero.png  (editor + live preview — the main shot)
- Open the **Welcome** note. Click **Toggle Preview** so the preview pane appears on the
  right. Both the editor (left) and rendered preview (right) should be visible, showing
  the heading, blockquote, rendered task checkboxes, highlighted code, and table.
- Run: `cap hero`

### 5. preview.png  (rich preview rendering)
- Keep preview ON. Open the **Diagrams & Math** note.
- The preview should render the **Mermaid flowchart**, the **math** (Gaussian integral +
  the display equations), and the highlighted Rust code. Give it a second to render the
  diagram. If you can, drag the divider between editor and preview slightly LEFT so the
  preview gets more room. Run: `cap preview`

### 6. quick-open.png  (Cmd-P fuzzy search)
- Click into the editor first, then press **Cmd-P**. A search palette appears over a
  dimmed editor. Type `roadmap`. You should see ranked results with folder breadcrumbs.
- Run: `cap quick-open`  — then press **Esc** to close the palette.

### 7. links.png  (wiki-link autocomplete)
- Open the **Welcome** note (preview can be on or off; OFF is cleaner). Click at the very
  end of the last line, press Return to make a new line, then type `[[di` .
- An autocomplete dropdown should appear suggesting **Diagrams & Math**. Run: `cap links`
- Then press **Esc** to dismiss the dropdown, and press **Cmd-Z** a few times to undo the
  `[[di` you typed (so the demo file is left clean).

### 8. typography.png  (the Typography sheet)
- Click the **Typography** (AA) toolbar button. A settings sheet slides down with controls
  for font, size, line height, and paragraph spacing.
- Run: `cap typography`  (the helper grabs the frontmost window = the sheet).
- Close the sheet (click Done / press Esc).

### 9. ai.png  (Bring-your-own-model AI settings)
- Click the **AI** (sparkles) toolbar button, then choose **AI Settings…** from the menu.
  A sheet appears with fields for endpoint, model, and API key (and a Test Connection
  button). Leave the fields blank/as-is — do NOT enter a real key.
- Run: `cap ai`  — then close the sheet.

## When done
You should have 9 files in ~/Projects/aaronbassett/emend/docs/screenshots/:
hero.png, editor.png, preview.png, workspace.png, quick-open.png, links.png,
info-sidebar.png, typography.png, ai.png

Quickly open the folder (or run `ls -la ~/Projects/aaronbassett/emend/docs/screenshots/`)
and confirm all 9 exist and look correct. Re-take any that show a stray cursor, an open
menu, an empty/wrong note, or an unrendered diagram. Report which shots you captured and
any you couldn't.
```
