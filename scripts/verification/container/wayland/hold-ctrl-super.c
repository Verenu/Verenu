#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>
#include <wayland-client.h>
#include <xkbcommon/xkbcommon.h>
#include "virtual-keyboard-client.h"

static struct wl_seat *seat;
static struct zwp_virtual_keyboard_manager_v1 *manager;

static void global(void *data, struct wl_registry *registry, uint32_t name, const char *iface, uint32_t version) {
    (void)data;
    if (!strcmp(iface, wl_seat_interface.name))
        seat = wl_registry_bind(registry, name, &wl_seat_interface, version < 7 ? version : 7);
    else if (!strcmp(iface, zwp_virtual_keyboard_manager_v1_interface.name))
        manager = wl_registry_bind(registry, name, &zwp_virtual_keyboard_manager_v1_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) { (void)data; (void)registry; (void)name; }
static const struct wl_registry_listener listener = { global, removed };

static int send_key(struct wl_display *display, struct zwp_virtual_keyboard_v1 *keyboard,
                    struct xkb_state *state, xkb_keycode_t key, int down) {
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    uint32_t ms = (uint32_t)((uint64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000);
    zwp_virtual_keyboard_v1_key(keyboard, ms, key - 8, down ? 1 : 0);
    xkb_state_update_key(state, key, down ? XKB_KEY_DOWN : XKB_KEY_UP);
    zwp_virtual_keyboard_v1_modifiers(keyboard,
        xkb_state_serialize_mods(state, XKB_STATE_MODS_DEPRESSED),
        xkb_state_serialize_mods(state, XKB_STATE_MODS_LATCHED),
        xkb_state_serialize_mods(state, XKB_STATE_MODS_LOCKED),
        xkb_state_serialize_layout(state, XKB_STATE_LAYOUT_EFFECTIVE));
    return wl_display_roundtrip(display);
}

int main(int argc, char **argv) {
    const char *home = getenv("HOME");
    if (!home || strcmp(home, "/home/agent") || access("/dev/snd", F_OK) == 0 ||
        !getenv("HYPRLAND_INSTANCE_SIGNATURE")) {
        fprintf(stderr, "Run only inside an isolated agent desktop\n");
        return 2;
    }
    if (argc != 2) { fprintf(stderr, "Usage: hold-ctrl-super milliseconds\n"); return 2; }
    char *end;
    long ms = strtol(argv[1], &end, 10);
    if (*end || ms < 50 || ms > 5000) { fprintf(stderr, "Duration must be 50–5000 ms\n"); return 2; }
    struct wl_display *display = wl_display_connect(NULL);
    if (!display) { fprintf(stderr, "No isolated Wayland display\n"); return 1; }
    struct wl_registry *registry = wl_display_get_registry(display);
    wl_registry_add_listener(registry, &listener, NULL);
    if (wl_display_roundtrip(display) < 0 || !seat || !manager) { fprintf(stderr, "Virtual keyboard is unavailable\n"); return 1; }
    struct xkb_context *context = xkb_context_new(XKB_CONTEXT_NO_FLAGS);
    struct xkb_rule_names names = { .layout = "us" };
    struct xkb_keymap *map = context ? xkb_keymap_new_from_names(context, &names, XKB_KEYMAP_COMPILE_NO_FLAGS) : NULL;
    struct xkb_state *state = map ? xkb_state_new(map) : NULL;
    char *text = map ? xkb_keymap_get_as_string(map, XKB_KEYMAP_FORMAT_TEXT_V1) : NULL;
    xkb_keycode_t ctrl = map ? xkb_keymap_key_by_name(map, "LCTL") : XKB_KEYCODE_INVALID;
    xkb_keycode_t super = map ? xkb_keymap_key_by_name(map, "LWIN") : XKB_KEYCODE_INVALID;
    if (!state || !text || ctrl != 37 || super != 133) { fprintf(stderr, "Expected evdev keymap unavailable\n"); return 1; }
    int fd = memfd_create("agent-keymap", MFD_CLOEXEC);
    size_t len = strlen(text) + 1;
    if (fd < 0 || write(fd, text, len) != (ssize_t)len) { fprintf(stderr, "Could not create keymap: %s\n", strerror(errno)); return 1; }
    struct zwp_virtual_keyboard_v1 *keyboard = zwp_virtual_keyboard_manager_v1_create_virtual_keyboard(manager, seat);
    zwp_virtual_keyboard_v1_keymap(keyboard, WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1, fd, (uint32_t)len);
    if (wl_display_roundtrip(display) < 0) return 1;
    close(fd);
    sigset_t stop_signal;
    sigemptyset(&stop_signal);
    sigaddset(&stop_signal, SIGTERM);
    if (sigprocmask(SIG_BLOCK, &stop_signal, NULL) < 0) return 1;
    if (send_key(display, keyboard, state, ctrl, 1) < 0 || send_key(display, keyboard, state, super, 1) < 0) return 1;
    struct timespec delay = { .tv_sec = ms / 1000, .tv_nsec = (ms % 1000) * 1000000 };
    while (nanosleep(&delay, &delay) < 0 && errno == EINTR) {}
    int result = send_key(display, keyboard, state, super, 0) < 0 || send_key(display, keyboard, state, ctrl, 0) < 0;
    struct timespec grace = { .tv_sec = 25, .tv_nsec = 0 };
    if (sigtimedwait(&stop_signal, NULL, &grace) < 0 && errno != EAGAIN) result = 1;
    zwp_virtual_keyboard_v1_destroy(keyboard);
    wl_display_roundtrip(display);
    free(text);
    xkb_state_unref(state); xkb_keymap_unref(map); xkb_context_unref(context);
    wl_registry_destroy(registry); wl_display_disconnect(display);
    return result;
}
