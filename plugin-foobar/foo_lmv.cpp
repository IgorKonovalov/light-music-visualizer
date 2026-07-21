// foo_lmv — foobar2000 visualization component for light-music-visualizer.
//
// A thin shim per ADR-0001: pulls PCM from foobar's visualisation_stream,
// forwards it across lmv-core's C ABI, and hosts the core's wgpu output in a
// plain Win32 window. All logic lives in the Rust core; this file only
// bridges foobar2000 conventions to the ABI in core/include/lmv_core.h.
//
// Threading: everything here runs on the foobar2000 main thread (menu
// command, window messages, render timer), which trivially satisfies the
// ABI's two-role threading contract.

#include "SDK/foobar2000.h"

#include <windows.h>

#include "lmv_core.h"

// foobar2000 x64 uses 64-bit audio_sample; lmv-core takes f32, so chunks are
// converted through a fixed buffer on the way in (see push_converted).

DECLARE_COMPONENT_VERSION(
    "Light Music Visualizer", "0.1.0",
    "Light Music Visualizer\n"
    "Spectrum, pulse and starfield scenes rendered by the shared lmv-core "
    "Rust engine (wgpu).\n"
    "Space cycles scenes.");
VALIDATE_COMPONENT_FILENAME("foo_lmv.dll");

namespace {

// GUIDs owned by this component - never reuse.
constexpr GUID kGuidLmvMenu = {
    0x8f7c2a1e, 0x94d3, 0x4b6a, {0x9c, 0x1f, 0x27, 0x5e, 0x88, 0x3a, 0xd4, 0x61}};

constexpr UINT_PTR kRenderTimer = 1;
// ~66 fps pump; actual pacing is vsync inside the core's present.
constexpr UINT kRenderTimerMs = 15;
// Read this far behind "now": visualisation data close to the playback head
// may not be decoded yet.
constexpr double kReadBehindSec = 0.05;
constexpr wchar_t kWindowClass[] = L"lmv_foobar_window";

// Singleton visualizer window state (main thread only).
HWND g_hwnd = nullptr;
LmvHandle *g_handle = nullptr;
visualisation_stream::ptr g_stream;
double g_cursor = 0.0;
uint32_t g_rate = 0;
uint16_t g_channels = 0;

void destroy_handle() {
    if (g_handle) {
        lmv_free(g_handle);
        g_handle = nullptr;
    }
    g_rate = 0;
    g_channels = 0;
}

// (Re)create the core handle for a stream format and attach the window.
// Called with the default format at open so scenes render even in silence,
// then again whenever the track's format differs.
void ensure_handle(uint32_t rate, uint16_t channels) {
    if (g_handle != nullptr && rate == g_rate && channels == g_channels) return;
    destroy_handle();
    LmvHandle *handle = lmv_create(rate, channels);
    if (handle == nullptr) return; // format outside core bounds - skip
    RECT rc = {};
    GetClientRect(g_hwnd, &rc);
    const uint32_t w = static_cast<uint32_t>(rc.right - rc.left);
    const uint32_t h = static_cast<uint32_t>(rc.bottom - rc.top);
    if (lmv_attach_window(handle, g_hwnd, w ? w : 1, h ? h : 1) != LMV_OK) {
        lmv_free(handle);
        return;
    }
    g_handle = handle;
    g_rate = rate;
    g_channels = channels;
}

// Convert audio_sample (double on x64 builds) to the f32 the ABI takes,
// through a fixed buffer - no per-tick allocation.
void push_converted(const audio_sample *data, size_t total, unsigned channels) {
    static float conv[8192];
    const size_t cap = (sizeof(conv) / sizeof(float)) / channels * channels;
    size_t off = 0;
    while (off < total && cap != 0) {
        const size_t n = (total - off < cap) ? (total - off) : cap;
        for (size_t i = 0; i < n; ++i) {
            conv[i] = static_cast<float>(data[off + i]);
        }
        lmv_push_samples(g_handle, conv, static_cast<uint32_t>(n));
        off += n;
    }
}

void pump_audio() {
    if (g_stream.is_empty()) return;
    double now = 0.0;
    if (!g_stream->get_absolute_time(now)) return;
    const double end = now - kReadBehindSec;
    // Resync after open, seek, or a long stall - never chase a huge backlog.
    if (g_cursor <= 0.0 || g_cursor > end || end - g_cursor > 0.5) {
        g_cursor = end;
        return;
    }
    if (end <= g_cursor) return;

    audio_chunk_impl chunk;
    if (g_stream->get_chunk_absolute(chunk, g_cursor, end - g_cursor)) {
        const unsigned rate = chunk.get_sample_rate();
        const unsigned channels = chunk.get_channels();
        const t_size samples = chunk.get_sample_count() * channels;
        if (rate != 0 && channels != 0 && samples != 0) {
            ensure_handle(static_cast<uint32_t>(rate),
                          static_cast<uint16_t>(channels));
            if (g_handle != nullptr) {
                push_converted(chunk.get_data(), samples, channels);
            }
        }
        g_cursor += chunk.get_duration();
    } else {
        g_cursor = end; // paused/stopped - keep the cursor near the head
    }
}

LRESULT CALLBACK wnd_proc(HWND wnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
        case WM_TIMER:
            if (wp == kRenderTimer) {
                pump_audio();
                if (g_handle != nullptr) lmv_render(g_handle);
                return 0;
            }
            break;
        case WM_SIZE:
            if (g_handle != nullptr && LOWORD(lp) != 0 && HIWORD(lp) != 0) {
                lmv_resize(g_handle, LOWORD(lp), HIWORD(lp));
            }
            return 0;
        case WM_KEYDOWN:
            if (wp == VK_SPACE && g_handle != nullptr) {
                lmv_cycle_scene(g_handle);
                return 0;
            }
            break;
        case WM_ERASEBKGND:
            return 1; // the core repaints every frame
        case WM_CLOSE:
            DestroyWindow(wnd);
            return 0;
        case WM_DESTROY:
            KillTimer(wnd, kRenderTimer);
            destroy_handle();
            g_stream.release();
            g_cursor = 0.0;
            g_hwnd = nullptr;
            return 0;
        default:
            break;
    }
    return DefWindowProcW(wnd, msg, wp, lp);
}

void open_window() {
    if (g_hwnd != nullptr) {
        SetForegroundWindow(g_hwnd);
        return;
    }
    static bool class_registered = false;
    if (!class_registered) {
        WNDCLASSW wc = {};
        wc.lpfnWndProc = wnd_proc;
        wc.hInstance = core_api::get_my_instance();
        wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
        wc.lpszClassName = kWindowClass;
        if (RegisterClassW(&wc) == 0) return;
        class_registered = true;
    }
    g_hwnd = CreateWindowExW(0, kWindowClass, L"Light Music Visualizer",
                             WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT,
                             CW_USEDEFAULT, 1024, 640, core_api::get_main_window(),
                             nullptr, core_api::get_my_instance(), nullptr);
    if (g_hwnd == nullptr) return;

    visualisation_manager::get()->create_stream(g_stream, 0);
    g_cursor = 0.0;
    // Default format so visuals run before (or without) playback; swapped
    // out automatically when the first chunk reports the real format.
    ensure_handle(48000, 2);
    SetTimer(g_hwnd, kRenderTimer, kRenderTimerMs, nullptr);
}

class mainmenu_commands_lmv : public mainmenu_commands {
public:
    t_uint32 get_command_count() override { return 1; }
    GUID get_command(t_uint32) override { return kGuidLmvMenu; }
    void get_name(t_uint32, pfc::string_base &out) override {
        out = "Light Music Visualizer";
    }
    bool get_description(t_uint32, pfc::string_base &out) override {
        out = "Opens the Light Music Visualizer window (Space cycles scenes).";
        return true;
    }
    GUID get_parent() override { return mainmenu_groups::view; }
    void execute(t_uint32, service_ptr_t<service_base>) override { open_window(); }
};

mainmenu_commands_factory_t<mainmenu_commands_lmv> g_mainmenu_factory;

// Tear the window down before the app finishes shutting down.
class initquit_lmv : public initquit {
public:
    void on_quit() override {
        if (g_hwnd != nullptr) DestroyWindow(g_hwnd);
    }
};

initquit_factory_t<initquit_lmv> g_initquit_factory;

} // namespace
