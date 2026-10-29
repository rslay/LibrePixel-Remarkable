// librepixel-ink: live pen ink and eraser through xochitl's own pen path while LibrePixel
// arms it (reMarkable 2).
//
// LibrePixel rewrites ARM_FILE every second while the pen may draw:
// "on x y w h size eraserTool eraseRadius ox oy zoom brush grid" (screen px; ox/oy = where
// sprite pixel (0,0) sits on screen, zoom = screen px per sprite pixel, brush in sprite pixels,
// grid = 1 when the editor draws grid lines) or "off"; a stale file counts as off.
//
// With the last five fields the editor drives the preview itself over CMD_FIFO: one line
// "fill x y w h c" per block it just decided to paint (screen px, c = 1 white / 0 black) and
// "end" when the stroke is over; this extension only puts those pixels on the panel through
// xochitl's fast pen path, leaving grid lines alone, so the preview is exactly what the editor
// paints. Without those fields (an older editor) it reads the pen itself and draws a round
// line. Either way the ink is only a preview: the editor still draws the real pixels and
// repaints the area.
//
// Derived from RemarkableAnimator's extension/animator-ink.c, Copyright (c) 2026 David @
// TheoryStudios, MIT License (see LICENSE.animator next to this file). Changes: arm/status
// file names, the build table, the status file the editor reads, the command pipe.
//
// It calls two stripped xochitl functions by address, so it only runs on builds listed in
// BUILDS (matched by GNU build id) and otherwise stays off. Adding a build is one table entry.
#define _GNU_SOURCE
#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <link.h>
#include <linux/input.h>
#include <poll.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define ARM_FILE "/tmp/librepixel-ink"
#define STATUS_FILE "/tmp/librepixel-ink.status"
#define CMD_FIFO "/tmp/librepixel-ink.cmd"
#define ARM_STALE_S 3       // s without a heartbeat from the editor: off
#define ARM_CHECK_MS 200
#define STATUS_MS 1000      // how often the status file is refreshed
#define SCREEN_W 1404
#define SCREEN_H 1872
#define WAVE_DU 1
#define PIXEL_PEN 8
#define FLAG_FASTDRAW 2

struct build {
    const char *name;
    unsigned char id[20];      // GNU build id of /usr/bin/xochitl
    uintptr_t fb_instance;     // EPFramebuffer* singleton, offset from the load base
    uintptr_t update_blocks;   // updateInBlocks(fb**, const QRegion&, waveform, pixelMode, flags, label)
    int fb_image;              // QImage (RGB32) inside EPFramebuffer
    int fb_image_ptr;          // QImage*, the current image (0 = the one at fb_image)
    int fb_mask;               // QImage (Grayscale8): the pen draws only where this is set
};

static const struct build BUILDS[] = {
    { "3.29.0.153",
      { 0xc1,0xa6,0x69,0x8e,0xc0,0x51,0x2d,0xdd,0x88,0xc2,0xba,0x5c,0xa7,0x13,0x39,0x09,0x88,0x97,0x80,0x56 },
      0x164f79c, 0x81093c, 0x58, 0x64, 0x68 },
};

typedef void (*UpdateFn)(void **fb, const void *region, int waveform, int pixelMode, int flags, const char *label);
typedef void (*RegionCtor)(void *self, int x, int y, int w, int h, int type);
typedef void (*RegionDtor)(void *self);
typedef int (*ImgInt)(const void *img);
typedef const unsigned char *(*ImgBits)(const void *img);

static uintptr_t base;
static const struct build *cur;
static RegionCtor rctor;
static RegionDtor rdtor;
static ImgInt format, width, height, stride;
static ImgBits bits;

static struct { int on, x, y, w, h, erase; float size, eraseR; int block, ox, oy, z, grid; } arm;

// Where to draw this stroke: the screen image and the pen mask
static struct { char *fb; uint32_t *px; unsigned char *mk; int ps, ms; } surf;

static void write_status(const char *what) {
    char tmp[96];
    snprintf(tmp, sizeof tmp, "%s.%d.tmp", STATUS_FILE, (int)getpid());
    FILE *f = fopen(tmp, "w");
    if (!f) return;
    if (cur) fprintf(f, "%s %s\n", what, cur->name); else fprintf(f, "%s\n", what);
    fclose(f);
    rename(tmp, STATUS_FILE); // readers never see a half-written file
}

// xovi's preload is inherited by every process xochitl spawns (shells, launched apps); only the
// real xochitl may ink, log or write the status file, or the children clobber it.
static int in_xochitl(void) {
    char exe[256];
    ssize_t n = readlink("/proc/self/exe", exe, sizeof exe - 1);
    if (n <= 0) return 0;
    exe[n] = 0;
    const char *slash = strrchr(exe, '/');
    return strcmp(slash ? slash + 1 : exe, "xochitl") == 0;
}

static int find_main(struct dl_phdr_info *info, size_t size, void *data) {
    (void)size; (void)data;
    base = info->dlpi_addr;
    for (int i = 0; i < info->dlpi_phnum; i++) {
        const ElfW(Phdr) *ph = &info->dlpi_phdr[i];
        if (ph->p_type != PT_NOTE) continue;
        const unsigned char *p = (const unsigned char *)(base + ph->p_vaddr), *end = p + ph->p_memsz;
        while (p + 12 <= end) {
            uint32_t namesz = *(uint32_t *)p, descsz = *(uint32_t *)(p + 4), type = *(uint32_t *)(p + 8);
            const unsigned char *desc = p + 12 + ((namesz + 3) & ~3u);
            if (type == 3 && descsz == 20) // NT_GNU_BUILD_ID
                for (size_t b = 0; b < sizeof BUILDS / sizeof *BUILDS; b++)
                    if (!memcmp(desc, BUILDS[b].id, 20)) cur = &BUILDS[b];
            p = desc + ((descsz + 3) & ~3u);
        }
    }
    return 1; // the first entry is the program itself
}

static long now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000L + ts.tv_nsec / 1000000L;
}

static void read_arm(void) {
    struct stat st;
    char buf[128] = "";
    int brush = 0;
    arm.on = 0;
    if (stat(ARM_FILE, &st) < 0 || time(NULL) - st.st_mtime > ARM_STALE_S) return;
    FILE *f = fopen(ARM_FILE, "r");
    if (!f) return;
    int n = fgets(buf, sizeof buf, f) ? sscanf(buf, "on %d %d %d %d %f %d %f %d %d %d %d %d", &arm.x, &arm.y, &arm.w, &arm.h,
                                                 &arm.size, &arm.erase, &arm.eraseR, &arm.ox, &arm.oy, &arm.z, &brush, &arm.grid) : 0;
    if (n < 7) { arm.erase = 0; arm.eraseR = 0; }
    arm.block = n >= 12 && arm.z > 0; // the editor drives the preview over the command pipe
    arm.on = n >= 5 && arm.w > 0 && arm.h > 0 && arm.size > 0;
    fclose(f);
}

// Find the screen image and pen mask, and check they are what this build is known to have
static int open_surface(void) {
    char *fb = *(char **)(base + cur->fb_instance);
    if (!fb) return 0;
    void *img = *(void **)(fb + cur->fb_image_ptr);
    if (!img) img = fb + cur->fb_image;
    void *mask = fb + cur->fb_mask;
    if (format(img) != 4 || width(img) != SCREEN_W || height(img) != SCREEN_H || format(mask) != 24
        || width(mask) != SCREEN_W || height(mask) != SCREEN_H) {
        fprintf(stderr, "[librepixel-ink] unexpected screen layout (fmt %d/%d), not drawing\n", format(img), format(mask));
        return 0;
    }
    surf.fb = fb;
    surf.px = (uint32_t *)bits(img);
    surf.mk = (unsigned char *)bits(mask);
    surf.ps = stride(img) / 4;
    surf.ms = stride(mask);
    return 1;
}

static int clampi(int v, int lo, int hi) { return v < lo ? lo : v > hi ? hi : v; }

static void grow(int box[4], int x0, int y0, int x1, int y1) {
    if (x0 < box[0]) box[0] = x0;
    if (y0 < box[1]) box[1] = y0;
    if (x1 > box[2]) box[2] = x1;
    if (y1 > box[3]) box[3] = y1;
}

// Paint a round-ended segment of width w, clipped to the armed canvas; returns its box
static void segment(float x0, float y0, float x1, float y1, float w, uint32_t ink, int box[4]) {
    float r = w / 2 < 0.75f ? 0.75f : w / 2;
    int bx0 = clampi((int)((x0 < x1 ? x0 : x1) - r), arm.x, arm.x + arm.w - 1);
    int by0 = clampi((int)((y0 < y1 ? y0 : y1) - r), arm.y, arm.y + arm.h - 1);
    int bx1 = clampi((int)((x0 > x1 ? x0 : x1) + r + 1), arm.x, arm.x + arm.w - 1);
    int by1 = clampi((int)((y0 > y1 ? y0 : y1) + r + 1), arm.y, arm.y + arm.h - 1);
    float dx = x1 - x0, dy = y1 - y0, len2 = dx * dx + dy * dy;
    for (int y = by0; y <= by1; y++)
        for (int x = bx0; x <= bx1; x++) {
            float px = x + 0.5f - x0, py = y + 0.5f - y0;
            float t = len2 > 0 ? (px * dx + py * dy) / len2 : 0;
            t = t < 0 ? 0 : t > 1 ? 1 : t;
            float ex = px - t * dx, ey = py - t * dy;
            if (ex * ex + ey * ey > r * r) continue;
            surf.px[y * surf.ps + x] = ink;
            surf.mk[y * surf.ms + x] = 0xFF;
        }
    box[0] = bx0; box[1] = by0; box[2] = bx1; box[3] = by1;
}

// A grid line runs along the first row and column of every sprite pixel on screen
// (a negative remainder is never zero, so this is right left of the origin too)
static int on_grid(int v, int origin) {
    return (v - origin) % arm.z == 0;
}

// Fill a screen rectangle the editor asked for, clipped to the armed canvas, keeping grid lines
static int fill(int x, int y, int w, int h, uint32_t ink, int box[4]) {
    int bx0 = clampi(x, arm.x, arm.x + arm.w), by0 = clampi(y, arm.y, arm.y + arm.h);
    int bx1 = clampi(x + w, arm.x, arm.x + arm.w), by1 = clampi(y + h, arm.y, arm.y + arm.h); // exclusive
    if (bx0 >= bx1 || by0 >= by1) return 0;
    for (int py = by0; py < by1; py++) {
        if (arm.grid && on_grid(py, arm.oy)) continue;
        for (int px = bx0; px < bx1; px++) {
            if (arm.grid && on_grid(px, arm.ox)) continue;
            surf.px[py * surf.ps + px] = ink;
            surf.mk[py * surf.ms + px] = 0xFF;
        }
    }
    box[0] = bx0; box[1] = by0; box[2] = bx1 - 1; box[3] = by1 - 1;
    return 1;
}

static void show(const int b[4]) {
    void *region[2] = {0};
    void *fbHolder = surf.fb;
    rctor(region, b[0], b[1], b[2] - b[0] + 1, b[3] - b[1] + 1, 0);
    ((UpdateFn)(base + cur->update_blocks))(&fbHolder, region, WAVE_DU, PIXEL_PEN, FLAG_FASTDRAW, " - librepixel ink");
    rdtor(region);
}

// The mask is the pen's scratch: clear what a stroke marked, once it is over
static void clear_mask(int sbox[4]) {
    if (sbox[2] < sbox[0] || sbox[3] < sbox[1]) return;
    for (int y = sbox[1]; y <= sbox[3]; y++) memset(surf.mk + y * surf.ms + sbox[0], 0, sbox[2] - sbox[0] + 1);
    sbox[0] = SCREEN_W; sbox[1] = SCREEN_H; sbox[2] = sbox[3] = -1;
}

#define HAS_BIT(bits, n) ((bits)[(n) / 8] >> ((n) % 8) & 1)

static int open_pen(void) {
    DIR *dir = opendir("/dev/input");
    struct dirent *e;
    int found = -1;
    while (dir && found < 0 && (e = readdir(dir))) {
        if (strncmp(e->d_name, "event", 5)) continue;
        char path[64];
        unsigned char abs_bits[ABS_MAX / 8 + 1] = {0}, key_bits[KEY_MAX / 8 + 1] = {0};
        snprintf(path, sizeof path, "/dev/input/%s", e->d_name);
        int fd = open(path, O_RDONLY | O_NONBLOCK);
        if (fd < 0) continue;
        ioctl(fd, EVIOCGBIT(EV_ABS, sizeof abs_bits), abs_bits);
        ioctl(fd, EVIOCGBIT(EV_KEY, sizeof key_bits), key_bits);
        if (HAS_BIT(abs_bits, ABS_PRESSURE) && HAS_BIT(key_bits, BTN_TOOL_PEN)) found = fd; else close(fd);
    }
    if (dir) closedir(dir);
    return found;
}

// The editor's command pipe. Opened read-write so poll never reports a hang-up between the
// editor's sessions.
static int open_cmd(void) {
    unlink(CMD_FIFO);
    if (mkfifo(CMD_FIFO, 0666) < 0 && errno != EEXIST) { fprintf(stderr, "[librepixel-ink] mkfifo: %s\n", strerror(errno)); return -1; }
    chmod(CMD_FIFO, 0666);
    return open(CMD_FIFO, O_RDWR | O_NONBLOCK);
}

// Apply the editor's command lines: fills now, end = the stroke is over
static int cmd_surface_ok;
static void run_cmd(const char *line, int sbox[4], int *updates) {
    int x, y, w, h, c;
    if (sscanf(line, "fill %d %d %d %d %d", &x, &y, &w, &h, &c) == 5) {
        if (!arm.on || !arm.block) return;
        if (!cmd_surface_ok) cmd_surface_ok = open_surface();
        if (!cmd_surface_ok) return;
        int b[4];
        if (fill(x, y, w, h, c ? 0xFFFFFFFFu : 0xFF000000u, b)) {
            show(b);
            (*updates)++;
            grow(sbox, b[0], b[1], b[2], b[3]);
        }
    } else if (!strncmp(line, "end", 3)) {
        if (cmd_surface_ok) clear_mask(sbox);
        if (*updates) fprintf(stderr, "[librepixel-ink] editor stroke: %d blocks\n", *updates);
        *updates = 0;
        cmd_surface_ok = 0;
    }
}

static void *pen_loop(void *arg) {
    (void)arg;
    int pen = open_pen();
    if (pen < 0) { fprintf(stderr, "[librepixel-ink] pen not found\n"); write_status("off no pen device"); return NULL; }
    struct input_absinfo ax = {0}, ay = {0}, ap = {0};
    ioctl(pen, EVIOCGABS(ABS_X), &ax);
    ioctl(pen, EVIOCGABS(ABS_Y), &ay);
    ioctl(pen, EVIOCGABS(ABS_PRESSURE), &ap);
    if (ax.maximum <= 0 || ay.maximum <= 0 || ap.maximum <= ap.minimum) {
        fprintf(stderr, "[librepixel-ink] odd pen ranges\n");
        write_status("off odd pen ranges");
        return NULL;
    }
    int cmd = open_cmd();
    fprintf(stderr, "[librepixel-ink] ready on %s: pen x 0..%d y 0..%d pressure ..%d, command pipe %s\n",
            cur->name, ax.maximum, ay.maximum, ap.maximum, cmd >= 0 ? "open" : "missing");
    write_status("ready");

    int rx = 0, ry = 0, rp = 0, touch = 0, rubber = 0;
    int stroke = 0, have_last = 0, sbox[4] = { SCREEN_W, SCREEN_H, -1, -1 }, cbox[4] = { SCREEN_W, SCREEN_H, -1, -1 };
    int updates = 0, cmd_updates = 0;
    float lx = 0, ly = 0;
    long last_arm = 0, last_status = now_ms(), t_stroke = 0, update_us = 0;
    char line[256];
    size_t len = 0;
    struct pollfd pfd[2] = { { pen, POLLIN, 0 }, { cmd, POLLIN, 0 } };
    for (;;) {
        poll(pfd, cmd >= 0 ? 2 : 1, ARM_CHECK_MS);
        long t = now_ms();
        if (!touch && t - last_arm >= ARM_CHECK_MS) { read_arm(); last_arm = t; }
        if (t - last_status >= STATUS_MS) { write_status("ready"); last_status = t; }

        // The editor's commands, line by line
        if (cmd >= 0 && (pfd[1].revents & POLLIN)) {
            char chunk[512];
            ssize_t n;
            while ((n = read(cmd, chunk, sizeof chunk)) > 0) {
                for (ssize_t i = 0; i < n; i++) {
                    if (chunk[i] == '\n') {
                        line[len] = 0;
                        run_cmd(line, cbox, &cmd_updates);
                        len = 0;
                    } else if (len < sizeof line - 1) line[len++] = chunk[i];
                }
            }
        }

        struct input_event ev[64];
        ssize_t n = read(pen, ev, sizeof ev);
        for (ssize_t i = 0; i < n / (ssize_t)sizeof *ev; i++) {
            struct input_event *e = &ev[i];
            if (e->type == EV_ABS) {
                if (e->code == ABS_X) rx = e->value;
                else if (e->code == ABS_Y) ry = e->value;
                else if (e->code == ABS_PRESSURE) rp = e->value;
            } else if (e->type == EV_KEY) {
                if (e->code == BTN_TOUCH) touch = e->value != 0;
                else if (e->code == BTN_TOOL_RUBBER) rubber = e->value != 0;
            } else if (e->type == EV_SYN && e->code == SYN_REPORT) {
                int erasing = rubber || arm.erase;
                if (arm.block) {
                    // The editor drives the preview; the pen is only watched to tidy up a
                    // stroke the editor never ended (it crashed or was closed mid-stroke).
                    if (!touch && cmd_surface_ok && cbox[2] >= cbox[0]) {
                        clear_mask(cbox);
                        cmd_surface_ok = 0;
                    }
                    continue;
                }
                if (touch && arm.on && (!erasing || arm.eraseR > 0)) {
                    if (!stroke) {
                        if (!open_surface()) { arm.on = 0; continue; }
                        stroke = 1; updates = 0; update_us = 0; t_stroke = now_ms();
                        sbox[0] = SCREEN_W; sbox[1] = SCREEN_H; sbox[2] = sbox[3] = -1;
                    }
                    // The panel's portrait frame: x along the digitizer's y, y against its x
                    float sx = ry * (float)SCREEN_W / ay.maximum, sy = (ax.maximum - rx) * (float)SCREEN_H / ax.maximum;
                    if (sx >= arm.x && sx < arm.x + arm.w && sy >= arm.y && sy < arm.y + arm.h) {
                        float p = (rp - ap.minimum) / (float)(ap.maximum - ap.minimum);
                        float w = erasing ? 2 * arm.eraseR : arm.size * (0.3f + 1.4f * (p > 1 ? 1 : p));
                        int b[4];
                        segment(have_last ? lx : sx, have_last ? ly : sy, sx, sy, w, erasing ? 0xFFFFFFFFu : 0xFF000000u, b);
                        struct timespec a, z;
                        clock_gettime(CLOCK_MONOTONIC, &a);
                        show(b);
                        clock_gettime(CLOCK_MONOTONIC, &z);
                        update_us += (z.tv_sec - a.tv_sec) * 1000000L + (z.tv_nsec - a.tv_nsec) / 1000;
                        updates++;
                        grow(sbox, b[0], b[1], b[2], b[3]);
                        lx = sx; ly = sy; have_last = 1;
                    } else have_last = 0;
                } else if (stroke && !touch) {
                    clear_mask(sbox);
                    fprintf(stderr, "[librepixel-ink] stroke: %d updates, %ld us per update call, %ld ms\n",
                            updates, updates ? update_us / updates : 0, now_ms() - t_stroke);
                    stroke = 0; have_last = 0;
                }
            }
        }
    }
    return NULL;
}

void _xovi_construct(void) {
    if (!in_xochitl()) return;
    dl_iterate_phdr(find_main, NULL);
    if (!cur) {
        fprintf(stderr, "[librepixel-ink] unknown xochitl build, staying off\n");
        write_status("off unknown xochitl build");
        return;
    }
    rctor = (RegionCtor)dlsym(RTLD_DEFAULT, "_ZN7QRegionC1EiiiiNS_10RegionTypeE");
    rdtor = (RegionDtor)dlsym(RTLD_DEFAULT, "_ZN7QRegionD1Ev");
    format = (ImgInt)dlsym(RTLD_DEFAULT, "_ZNK6QImage6formatEv");
    width = (ImgInt)dlsym(RTLD_DEFAULT, "_ZNK6QImage5widthEv");
    height = (ImgInt)dlsym(RTLD_DEFAULT, "_ZNK6QImage6heightEv");
    stride = (ImgInt)dlsym(RTLD_DEFAULT, "_ZNK6QImage12bytesPerLineEv");
    bits = (ImgBits)dlsym(RTLD_DEFAULT, "_ZNK6QImage4bitsEv");
    if (!rctor || !rdtor || !format || !width || !height || !stride || !bits) {
        fprintf(stderr, "[librepixel-ink] Qt symbols missing, staying off\n");
        write_status("off Qt symbols missing");
        return;
    }
    pthread_t t;
    pthread_create(&t, NULL, pen_loop, NULL);
    pthread_detach(t);
}
