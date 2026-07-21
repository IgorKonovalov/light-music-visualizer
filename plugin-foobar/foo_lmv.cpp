// foo_lmv — foobar2000 visualization component for light-music-visualizer.
//
// A thin shim per ADR-0001: pulls PCM from foobar's visualisation_stream,
// forwards it across lmv-core's C ABI, and hosts the core's wgpu output in a
// plain Win32 window. All logic lives in the Rust core; this file only
// bridges foobar2000 conventions to the ABI in core/include/lmv_core.h.
//
// Two entry points share ONE core instance: a View-menu pop-out window and a
// Default UI panel (ui_element). Both are "host windows" that claim a single
// global VizSession; the session holds the sole LmvHandle + visualisation
// stream + render timer, and only its current owner drives the core. A second
// host cannot create a second wgpu surface (the "lightweight / one surface"
// value). Placeholder painting for non-owners lands in a later phase.
//
// Threading: everything here runs on the foobar2000 main thread (menu
// command, window messages, render timer), which trivially satisfies the
// ABI's two-role threading contract.

#include "SDK/foobar2000.h"
#include "SDK/ui_element.h"

#include <windows.h>
#include <windowsx.h> // GET_X_LPARAM / GET_Y_LPARAM

#include "lmv_core.h"

// foobar2000 x64 uses 64-bit audio_sample; lmv-core takes f32, so chunks are
// converted through a fixed buffer on the way in (see push_converted).

DECLARE_COMPONENT_VERSION(
    "Light Music Visualizer", "0.1.0",
    "Light Music Visualizer\n"
    "Spectrum, pulse and starfield scenes rendered by the shared lmv-core "
    "Rust engine (wgpu).\n"
    "Dockable as a Default UI panel or opened from the View menu. "
    "Space cycles scenes.");
VALIDATE_COMPONENT_FILENAME("foo_lmv.dll");

namespace {

// GUIDs owned by this component - never reuse.
constexpr GUID kGuidLmvMenu = {
    0x8f7c2a1e, 0x94d3, 0x4b6a, {0x9c, 0x1f, 0x27, 0x5e, 0x88, 0x3a, 0xd4, 0x61}};
constexpr GUID kGuidLmvElement = {
    0x2d9b4f7c, 0x6e21, 0x4a83, {0xb5, 0x0c, 0x1a, 0x77, 0x3e, 0x08, 0xc2, 0x54}};

constexpr UINT_PTR kRenderTimer = 1;
// ~66 fps pump; actual pacing is vsync inside the core's present.
constexpr UINT kRenderTimerMs = 15;
// A non-owning host polls on this timer to take over once the session frees
// (owning panel removed, pop-out closed) and to keep its placeholder painted.
constexpr UINT_PTR kArbitrationTimer = 2;
constexpr UINT kArbitrationMs = 400;
// Context-menu command id (window-local; not a foobar menu GUID).
constexpr UINT kMenuNextScene = 1001;
// Read this far behind "now": visualisation data close to the playback head
// may not be decoded yet.
constexpr double kReadBehindSec = 0.05;
constexpr wchar_t kWindowClass[] = L"lmv_foobar_window";

// The single shared visualizer session (main thread only). Exactly one host
// window (pop-out or a panel) owns it at a time; only the owner holds the
// LmvHandle, stream and render timer, so there is only ever one wgpu surface.
struct VizSession {
    HWND owner = nullptr; // host window currently driving the core
    LmvHandle *handle = nullptr;
    visualisation_stream::ptr stream;
    double cursor = 0.0;
    uint32_t rate = 0;
    uint16_t channels = 0;

    void destroy_handle();
    void ensure_handle(uint32_t rate, uint16_t channels);
    void push_converted(const audio_sample *data, size_t total, unsigned channels);
    void pump();

    // Take ownership for `host` if the session is free. On success the core
    // handle + stream + render timer are live on `host`; returns false (no
    // core created) if another host already owns the session.
    bool claim(HWND host);
    // Release ownership held by `host` (no-op if `host` is not the owner),
    // freeing the handle + stream and stopping the timer.
    void release(HWND host);
};

VizSession g_session;

// Tracks the pop-out window independently of session ownership, so the View
// command can bump an existing pop-out and on_quit can tear it down.
HWND g_popup_hwnd = nullptr;

void VizSession::destroy_handle() {
    if (handle) {
        lmv_free(handle);
        handle = nullptr;
    }
    rate = 0;
    channels = 0;
}

// (Re)create the core handle for a stream format and attach the owner window.
// Called with the default format on claim so scenes render even in silence,
// then again whenever the track's format differs. Requires `owner` set.
void VizSession::ensure_handle(uint32_t new_rate, uint16_t new_channels) {
    if (handle != nullptr && new_rate == rate && new_channels == channels) return;
    destroy_handle();
    LmvHandle *h = lmv_create(new_rate, new_channels);
    if (h == nullptr) return; // format outside core bounds - skip
    RECT rc = {};
    GetClientRect(owner, &rc);
    const uint32_t w = static_cast<uint32_t>(rc.right - rc.left);
    const uint32_t ht = static_cast<uint32_t>(rc.bottom - rc.top);
    if (lmv_attach_window(h, owner, w ? w : 1, ht ? ht : 1) != LMV_OK) {
        lmv_free(h);
        return;
    }
    handle = h;
    rate = new_rate;
    channels = new_channels;
}

// Convert audio_sample (double on x64 builds) to the f32 the ABI takes,
// through a fixed buffer - no per-tick allocation.
void VizSession::push_converted(const audio_sample *data, size_t total,
                                unsigned chans) {
    static float conv[8192];
    const size_t cap = (sizeof(conv) / sizeof(float)) / chans * chans;
    size_t off = 0;
    while (off < total && cap != 0) {
        const size_t n = (total - off < cap) ? (total - off) : cap;
        for (size_t i = 0; i < n; ++i) {
            conv[i] = static_cast<float>(data[off + i]);
        }
        lmv_push_samples(handle, conv, static_cast<uint32_t>(n));
        off += n;
    }
}

void VizSession::pump() {
    if (stream.is_empty()) return;
    double now = 0.0;
    if (!stream->get_absolute_time(now)) return;
    const double end = now - kReadBehindSec;
    // Resync after open, seek, or a long stall - never chase a huge backlog.
    if (cursor <= 0.0 || cursor > end || end - cursor > 0.5) {
        cursor = end;
        return;
    }
    if (end <= cursor) return;

    audio_chunk_impl chunk;
    if (stream->get_chunk_absolute(chunk, cursor, end - cursor)) {
        const unsigned chunk_rate = chunk.get_sample_rate();
        const unsigned chunk_channels = chunk.get_channels();
        const t_size samples = chunk.get_sample_count() * chunk_channels;
        if (chunk_rate != 0 && chunk_channels != 0 && samples != 0) {
            ensure_handle(static_cast<uint32_t>(chunk_rate),
                          static_cast<uint16_t>(chunk_channels));
            if (handle != nullptr) {
                push_converted(chunk.get_data(), samples, chunk_channels);
            }
        }
        cursor += chunk.get_duration();
    } else {
        cursor = end; // paused/stopped - keep the cursor near the head
    }
}

bool VizSession::claim(HWND host) {
    if (owner != nullptr) return false; // another host drives the core
    if (stream.is_empty()) {
        visualisation_manager::get()->create_stream(stream, 0);
    }
    cursor = 0.0;
    owner = host; // ensure_handle attaches to the owner window
    // Default format so visuals run before (or without) playback; swapped
    // out automatically when the first chunk reports the real format.
    ensure_handle(48000, 2);
    if (handle == nullptr) {
        owner = nullptr; // create failed - stay free so another host may try
        stream.release();
        return false;
    }
    SetTimer(host, kRenderTimer, kRenderTimerMs, nullptr);
    return true;
}

void VizSession::release(HWND host) {
    if (owner != host) return;
    KillTimer(host, kRenderTimer);
    destroy_handle();
    stream.release();
    cursor = 0.0;
    owner = nullptr;
}

// Paint the "someone else owns the core" placeholder for a non-owning host.
void paint_placeholder(HWND wnd, HDC hdc) {
    RECT rc = {};
    GetClientRect(wnd, &rc);
    FillRect(hdc, &rc, static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH)));
    const wchar_t *msg = L"Light Music Visualizer is active in another window";
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, RGB(180, 180, 180));
    // Word-wrap, then vertically centre the wrapped block within the client.
    RECT measure = rc;
    DrawTextW(hdc, msg, -1, &measure, DT_CENTER | DT_WORDBREAK | DT_CALCRECT);
    RECT draw = rc;
    const LONG text_h = measure.bottom - measure.top;
    if (text_h < rc.bottom - rc.top) {
        draw.top = rc.top + ((rc.bottom - rc.top) - text_h) / 2;
    }
    DrawTextW(hdc, msg, -1, &draw, DT_CENTER | DT_WORDBREAK);
}

// Shared window procedure for both host kinds (pop-out top-level and panel
// child). The owner check gates every core call so a non-owning host never
// touches the handle; a non-owner runs an arbitration timer to claim the
// session once it frees and paints the placeholder meanwhile.
LRESULT CALLBACK wnd_proc(HWND wnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
        case WM_CREATE:
            if (!g_session.claim(wnd)) {
                // Another host owns the core - wait for it to free the session.
                SetTimer(wnd, kArbitrationTimer, kArbitrationMs, nullptr);
            }
            return 0;
        case WM_TIMER:
            if (wp == kRenderTimer) {
                if (g_session.owner == wnd) {
                    g_session.pump();
                    if (g_session.handle != nullptr) lmv_render(g_session.handle);
                }
                return 0;
            }
            if (wp == kArbitrationTimer) {
                // Session free? Take it over (claim starts the render timer)
                // and repaint to clear the placeholder.
                if (g_session.owner == nullptr && g_session.claim(wnd)) {
                    KillTimer(wnd, kArbitrationTimer);
                    InvalidateRect(wnd, nullptr, FALSE);
                }
                return 0;
            }
            break;
        case WM_SIZE:
            if (g_session.owner == wnd && g_session.handle != nullptr &&
                LOWORD(lp) != 0 && HIWORD(lp) != 0) {
                lmv_resize(g_session.handle, LOWORD(lp), HIWORD(lp));
            } else if (g_session.owner != wnd) {
                InvalidateRect(wnd, nullptr, FALSE); // re-centre the placeholder
            }
            return 0;
        case WM_KEYDOWN:
            if (wp == VK_SPACE && g_session.owner == wnd &&
                g_session.handle != nullptr) {
                lmv_cycle_scene(g_session.handle);
                return 0;
            }
            break;
        case WM_LBUTTONDOWN:
            SetFocus(wnd); // so a subsequent Space reaches this panel/window
            return 0;
        case WM_CONTEXTMENU: {
            // Owner-only: the right-click "Next scene" works without keyboard
            // focus; a placeholder (non-owner) host offers nothing.
            if (g_session.owner != wnd || g_session.handle == nullptr) break;
            POINT pt = {GET_X_LPARAM(lp), GET_Y_LPARAM(lp)};
            if (pt.x == -1 && pt.y == -1) { // keyboard-invoked: centre on window
                RECT rc = {};
                GetWindowRect(wnd, &rc);
                pt.x = (rc.left + rc.right) / 2;
                pt.y = (rc.top + rc.bottom) / 2;
            }
            HMENU menu = CreatePopupMenu();
            if (menu == nullptr) return 0;
            AppendMenuW(menu, MF_STRING, kMenuNextScene, L"Next scene");
            const int cmd =
                TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD, pt.x, pt.y,
                               0, wnd, nullptr);
            DestroyMenu(menu);
            if (cmd == kMenuNextScene && g_session.owner == wnd &&
                g_session.handle != nullptr) {
                lmv_cycle_scene(g_session.handle);
            }
            return 0;
        }
        case WM_PAINT:
            if (g_session.owner != wnd) {
                PAINTSTRUCT ps = {};
                HDC hdc = BeginPaint(wnd, &ps);
                paint_placeholder(wnd, hdc);
                EndPaint(wnd, &ps);
                return 0;
            }
            break; // owner: the core presents on its timer; DefWindowProc validates
        case WM_ERASEBKGND:
            return 1; // owner: core repaints; non-owner: WM_PAINT fills fully
        case WM_CLOSE:
            DestroyWindow(wnd); // pop-out only; panels are destroyed by the host
            return 0;
        case WM_DESTROY:
            KillTimer(wnd, kArbitrationTimer); // no-op if this host was the owner
            g_session.release(wnd); // frees the handle iff this host owned it
            if (wnd == g_popup_hwnd) g_popup_hwnd = nullptr; // allow reopening
            return 0;
        default:
            break;
    }
    return DefWindowProcW(wnd, msg, wp, lp);
}

// Register the shared window class once.
void ensure_window_class() {
    static bool registered = false;
    if (registered) return;
    WNDCLASSW wc = {};
    wc.lpfnWndProc = wnd_proc;
    wc.hInstance = core_api::get_my_instance();
    wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
    wc.lpszClassName = kWindowClass;
    if (RegisterClassW(&wc) != 0) registered = true;
}

// ---- Pop-out window (View menu) ----------------------------------------

void open_window() {
    if (g_popup_hwnd != nullptr) {
        SetForegroundWindow(g_popup_hwnd);
        return;
    }
    ensure_window_class();
    g_popup_hwnd = CreateWindowExW(
        0, kWindowClass, L"Light Music Visualizer",
        WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT, 1024, 640,
        core_api::get_main_window(), nullptr, core_api::get_my_instance(),
        nullptr);
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

// Tear the pop-out down before the app finishes shutting down. Panels are
// destroyed by the Default UI host; whichever owned the session releases it
// via WM_DESTROY, so the handle is freed exactly once.
class initquit_lmv : public initquit {
public:
    void on_quit() override {
        if (g_popup_hwnd != nullptr) DestroyWindow(g_popup_hwnd);
    }
};

initquit_factory_t<initquit_lmv> g_initquit_factory;

// ---- Default UI panel (ui_element) -------------------------------------

// One embedded panel instance: owns a WS_CHILD window parented into the
// layout. The window claims the shared session on WM_CREATE and releases it on
// WM_DESTROY, exactly like the pop-out, so no panel-specific core logic exists.
class lmv_ui_element_instance : public ui_element_instance {
public:
    lmv_ui_element_instance(HWND parent, ui_element_instance_callback_ptr callback)
        : m_callback(callback) {
        ensure_window_class();
        m_wnd = CreateWindowExW(0, kWindowClass, L"", WS_CHILD | WS_VISIBLE, 0, 0,
                                0, 0, parent, nullptr,
                                core_api::get_my_instance(), nullptr);
    }
    ~lmv_ui_element_instance() {
        if (m_wnd != nullptr) DestroyWindow(m_wnd);
    }

    fb2k::hwnd_t get_wnd() override { return m_wnd; }
    void set_configuration(ui_element_config::ptr) override {}
    ui_element_config::ptr get_configuration() override {
        return ui_element_config::g_create_empty(kGuidLmvElement);
    }
    GUID get_guid() override { return kGuidLmvElement; }
    GUID get_subclass() override {
        return ui_element_subclass_playback_visualisation;
    }

private:
    HWND m_wnd = nullptr;
    ui_element_instance_callback_ptr m_callback;
};

class lmv_ui_element : public ui_element {
public:
    GUID get_guid() override { return kGuidLmvElement; }
    GUID get_subclass() override {
        return ui_element_subclass_playback_visualisation;
    }
    void get_name(pfc::string_base &out) override {
        out = "Light Music Visualizer";
    }
    ui_element_instance_ptr instantiate(
        fb2k::hwnd_t parent, ui_element_config::ptr,
        ui_element_instance_callback_ptr callback) override {
        return new service_impl_t<lmv_ui_element_instance>(
            static_cast<HWND>(parent), callback);
    }
    ui_element_config::ptr get_default_configuration() override {
        return ui_element_config::g_create_empty(kGuidLmvElement);
    }
    ui_element_children_enumerator_ptr enumerate_children(
        ui_element_config::ptr) override {
        return nullptr;
    }
    bool get_description(pfc::string_base &out) override {
        out = "Audio-reactive visuals (spectrum, pulse, starfield) from "
              "lmv-core. Space cycles scenes.";
        return true;
    }
};

service_factory_single_t<lmv_ui_element> g_lmv_ui_element_factory;

} // namespace
