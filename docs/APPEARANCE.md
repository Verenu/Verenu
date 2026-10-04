# Appearance and settings

Open **Settings -> General** to change how Verenu looks. Appearance preferences are stored locally and apply to the main window and dictation pill.

## Theme

Choose **System**, **Light**, or **Dark** under Appearance, or pick a theme from the gallery.

- **System** follows the current Windows or macOS appearance. On Linux with an Omarchy theme, it follows that theme.
- **Light** uses neutral near-white backgrounds.
- **Dark** uses neutral charcoal backgrounds.

- A **theme** paints the whole app with colors you choose. See below.

The current palette avoids the older cream light theme and orange-brown dark theme. Backgrounds, dividers, and text stay neutral so the accent color remains distinct.

### Themes

Under Appearance, **System**, **Light**, and **Dark** are preview cards. Below them the **Themes** gallery lists built-in palettes (Catppuccin, Tokyo Night, Nord, Gruvbox, Solarized) and your saved themes. Click a card to apply it. Themes are available on Windows, macOS, and Linux.

**Create theme** (or the pencil on any card) opens the theme editor in the bottom-right corner. It is not a dialog: it stays open while you move around the app, and every color you change previews live across the whole window.

- Start with **Background** and **Accent**. Text and surface colors are derived automatically. Light or dark is chosen from the background's brightness, which also sets the native title bar, tray icon, and window controls.
- **Advanced** exposes **Text**, **Sidebar**, and **Surface** (cards, menus, dialogs). Sidebar, Surface, and Accent are optional; blank values use derived colors. Palettes with explicit overrides open Advanced automatically.
- **Save theme** stores it under its name and applies it. **Cancel**, Escape, or the close button restore exactly what was applied before; with unsaved changes the editor asks before discarding. Editing a built-in palette saves a copy.
- Saved themes can be reopened, edited, and deleted. Deleting the applied theme leaves its colors in place until you pick another.

If a save fails, the editor stays open with your draft and shows the error; nothing is half-applied.

Borders, muted text, hover states, overlays, shadows, and the dictation pill are all derived from these colors. Success, warning, and error colors stay Verenu's own. The palette and the saved theme list stay on the device and are included in backups, but are not synced.

### Omarchy theme

On Linux with Omarchy, System reads the active theme's `colors.toml` and applies it to surfaces, text, borders, the dictation pill, tray icons, and the default accent. Text stays neutral (white on dark themes, near-black on light ones) rather than taking the theme's tint. Error, success, and warning colors come from the theme's red, green, and yellow when those are vivid enough to read as status; monochrome themes keep Verenu's own so status stays visible. `omarchy theme set` applies live without restarting Verenu.

The accent defaults to the Omarchy theme's accent (or your text color for Custom), and an accent you pick still takes precedence. Custom is the one mode that uses the Text color you type for text.

## Accent color

Accent color controls actions, selection indicators, focus rings, and status details. Choose a preset or enter a six-digit hex color in the accent picker. Verenu derives the related soft background, readable text, and foreground colors for both light and dark themes.

Reset the picker to remove the custom value and restore Verenu's neutral default accent: black in light mode and white in dark mode. Changing the accent does not tint the neutral page backgrounds.

## Finding a setting

Use the search field at the top of the settings sidebar to find a preference by name or related term. Selecting a result opens its section, scrolls the preference into view, and briefly marks its position at the left edge.
