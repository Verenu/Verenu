# Appearance and settings

Open **Settings -> General** to change how Verenu looks. Appearance preferences are stored locally and apply to the main window and dictation pill.

## Theme

Choose **System**, **Light**, **Dark**, or **Custom** under Appearance.

- **System** follows the current Windows or macOS appearance. On Linux with an Omarchy theme, it follows that theme.
- **Light** uses neutral near-white backgrounds.
- **Dark** uses neutral charcoal backgrounds.

- **Custom** paints the whole app with colors you enter as hex codes. See below.

The current palette avoids the older cream light theme and orange-brown dark theme. Backgrounds, dividers, and text stay neutral so the accent color remains distinct.

### Custom colors

Custom is available on Windows, macOS, and Linux. Pick a preset (Catppuccin, Tokyo Night, Nord, Gruvbox, Solarized) or type your own six-digit hex codes:

- **Background** and **Text** are required. Light or dark is chosen from the background's brightness, which also sets the native title bar, tray icon, and window controls.
- **Sidebar** and **Surface** (cards, menus, dialogs) are optional. When blank they are derived from the background and text.

Borders, muted text, hover states, overlays, shadows, and the dictation pill are all derived from these colors, and changes apply live. **Reset colors** returns to the starting palette. Success, warning, and error colors stay Verenu's own. The palette is saved with your settings and included in backups. Like the other appearance settings, it stays on the device and is not synced.

### Omarchy theme

On Linux with Omarchy, System reads the active theme's `colors.toml` and applies it to surfaces, text, borders, the dictation pill, tray icons, and the default accent. Text stays neutral (white on dark themes, near-black on light ones) rather than taking the theme's tint. Error, success, and warning colors come from the theme's red, green, and yellow when those are vivid enough to read as status; monochrome themes keep Verenu's own so status stays visible. `omarchy theme set` applies live without restarting Verenu.

The accent defaults to the Omarchy theme's accent (or your text color for Custom), and an accent you pick still takes precedence. Custom is the one mode that uses the Text color you type for text.

## Accent color

Accent color controls actions, selection indicators, focus rings, and status details. Choose a preset or enter a six-digit hex color in the accent picker. Verenu derives the related soft background, readable text, and foreground colors for both light and dark themes.

Reset the picker to remove the custom value and restore Verenu's neutral default accent: black in light mode and white in dark mode. Changing the accent does not tint the neutral page backgrounds.

## Finding a setting

Use the search field at the top of the settings sidebar to find a preference by name or related term. Selecting a result opens its section, scrolls the preference into view, and briefly marks its position at the left edge.
