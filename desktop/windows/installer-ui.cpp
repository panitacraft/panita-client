// Presentation-only NSIS plugin. No installation, filesystem or account logic.
// The parameter-free export follows NSIS's Unicode plugin ABI. /NOUNLOAD keeps
// the subclass procedures resident until the installer process exits.
#define UNICODE
#define _UNICODE
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <windowsx.h>
#include <commctrl.h>
#include <objidl.h>
#include <gdiplus.h>
#include <cwctype>
#include <algorithm>
#include <array>
#include <string>
#include <vector>
#include "installer-components.h"

namespace {
constexpr COLORREF background = RGB(20, 20, 20);
constexpr COLORREF card = RGB(27, 27, 27);
constexpr COLORREF accent_color = RGB(84, 166, 54);
HWND root;
HFONT body_font;
HFONT caption_font;
HFONT option_font;
HBRUSH log_brush;
HBRUSH background_brush;
HWND detail_source;
HWND wrapped_log;
bool log_sync_queued;
constexpr UINT sync_log_message = WM_APP + 42;
constexpr UINT apply_message = WM_APP + 43;
constexpr UINT_PTR controls_timer = 0x5041;
HWND current_footer;
HWND tooltips;
HWND hovered_button;
ULONG_PTR graphics_token;
Gdiplus::Image* brand_image;
constexpr int shell_width = 500;
constexpr int content_width = shell_width - 56;
int shell_height = 460;
bool details_open;
using Controls = std::array<HWND, 28>;
void apply_presentation(HWND parent);
HWND header_control(HWND page, const wchar_t* type, const wchar_t* text, int tag, DWORD style);
int phase() { return static_cast<int>(reinterpret_cast<INT_PTR>(GetPropW(root, L"PanitaPhase"))); }

int scale(int value) { return MulDiv(value, static_cast<int>(GetDpiForWindow(root)), 96); }
int role(HWND window) {
  return static_cast<int>(reinterpret_cast<INT_PTR>(GetPropW(window, L"PanitaRole")));
}
void draw_logo(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  HBRUSH brush = CreateSolidBrush(card);
  FillRect(dc, &bounds, brush);
  DeleteObject(brush);
  if (brand_image) {
    Gdiplus::Graphics graphics(dc);
    graphics.SetInterpolationMode(Gdiplus::InterpolationModeHighQualityBicubic);
    graphics.SetPixelOffsetMode(Gdiplus::PixelOffsetModeHalf);
    graphics.DrawImage(brand_image, 0, 0, static_cast<int>(bounds.right), static_cast<int>(bounds.bottom));
  } else {
    const auto icon = reinterpret_cast<HICON>(SendMessageW(window, STM_GETICON, 0, 0));
    DrawIconEx(dc, 0, 0, icon, bounds.right, bounds.bottom, 0, nullptr, DI_NORMAL);
  }
}
bool visible(HWND window) {
  return window && (GetWindowLongPtrW(window, GWL_STYLE) & WS_VISIBLE);
}
void place(HWND window, int x, int y, int width, int height) {
  if (window) SetWindowPos(window, nullptr, scale(x), scale(y), scale(width),
      scale(height), SWP_NOZORDER | SWP_NOACTIVATE);
}
std::wstring window_text(HWND window) {
  const int length = (std::min)(GetWindowTextLengthW(window), 1048576);
  std::wstring text(static_cast<size_t>(length) + 1, L'\0');
  GetWindowTextW(window, text.data(), length + 1);
  text.resize(static_cast<size_t>(length));
  return text;
}
int text_height(HWND window, int width) {
  auto text = window_text(window);
  HDC dc = GetDC(window);
  HGDIOBJ old = SelectObject(dc, reinterpret_cast<HFONT>(SendMessageW(window, WM_GETFONT, 0, 0)));
  RECT bounds{0, 0, scale(width), 0};
  DrawTextW(dc, text.c_str(), -1, &bounds, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
  SelectObject(dc, old);
  ReleaseDC(window, dc);
  const int dpi = static_cast<int>(GetDpiForWindow(root));
  return (std::max)(18, (static_cast<int>(bounds.bottom) * 96 + dpi - 1) / dpi);
}
BOOL CALLBACK collect_roles(HWND window, LPARAM data) {
  auto& controls = *reinterpret_cast<Controls*>(data);
  const int id = role(window);
  if (id > 0 && id < static_cast<int>(controls.size())) controls[static_cast<size_t>(id)] = window;
  return TRUE;
}
void resize_shell(HWND page, int content_bottom = 340) {
  shell_height = (std::max)(460, content_bottom + 100);
  SetWindowPos(root, nullptr, 0, 0, scale(shell_width), scale(shell_height),
      SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
  place(page, 28, 16, content_width, shell_height - 32);
  HRGN rounded = CreateRoundRectRgn(0, 0, scale(shell_width) + 1, scale(shell_height) + 1, scale(20), scale(20));
  if (!SetWindowRgn(root, rounded, TRUE)) DeleteObject(rounded);
}
void position_header(const Controls& controls) {
  place(controls[1], 0, 0, 40, 40);
  place(controls[2], 52, 0, 280, 40);
  place(controls[3], content_width - 72, 3, 34, 34);
  place(controls[4], content_width - 34, 3, 34, 34);
  place(controls[5], 202, 8, 104, 24);
  ShowWindow(controls[5], SW_HIDE);
}
// All notices share the same inset, icon gutter and measured multiline layout.
int layout_notice(HWND page, HWND frame, HWND text, int cursor) {
  if (!frame) frame = header_control(page, L"STATIC", L"", 27, WS_CLIPSIBLINGS);
  ShowWindow(frame, SW_SHOWNA);
  const int height = text_height(text, content_width - 60);
  place(frame, 0, cursor, content_width, height + 32);
  SetWindowPos(frame, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
  place(text, 44, cursor + 16, content_width - 60, height);
  return cursor + height + 32;
}
void compact_page(const Controls& controls) {
  if (!controls[6] || !controls[7]) return;
  HWND page = GetParent(controls[6]);
  position_header(controls);
  int cursor = 84;
  auto row = [&](int id, int gap, int minimum, int width = content_width) {
    HWND control = controls[static_cast<size_t>(id)];
    if (!visible(control)) return;
    const int height = (std::max)(minimum, text_height(control, width));
    place(control, 0, cursor, content_width, height);
    cursor += height + gap;
  };
  row(6, 12, 34);
  // A page with no sentence under its title leaves no gap.
  ShowWindow(controls[7], window_text(controls[7]).find_first_not_of(L" \t\r\n") == std::wstring::npos ? SW_HIDE : SW_SHOWNA);
  HWND warning = controls[27];
  const bool error = GetPropW(controls[7], L"PanitaError") != nullptr;
  if (error) {
    cursor = layout_notice(page, warning, controls[7], cursor) + 22;
  } else {
    if (warning) ShowWindow(warning, SW_HIDE);
    row(7, 22, 18);
  }
  if (visible(controls[8]) && visible(controls[9])) {
    const int inset = 44;
    const int height = text_height(controls[9], content_width - inset - 16);
    place(controls[8], 0, cursor, content_width, height + 32);
    place(controls[9], inset, cursor + 16, content_width - inset - 16, height);
    cursor += height + 50;
  }
  row(24, 8, 18);
  if (visible(controls[10])) {
    HWND frame = controls[25];
    if (!frame) frame = header_control(page, L"STATIC", L"", 25, WS_CLIPSIBLINGS);
    place(frame, 0, cursor, content_width - 110, 36);
    SetWindowPos(frame, HWND_BOTTOM, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    SetWindowLongPtrW(controls[10], GWL_STYLE, GetWindowLongPtrW(controls[10], GWL_STYLE) & ~WS_BORDER);
    SetWindowLongPtrW(controls[10], GWL_EXSTYLE, GetWindowLongPtrW(controls[10], GWL_EXSTYLE) & ~WS_EX_CLIENTEDGE);
    SetWindowPos(controls[10], nullptr, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
    place(controls[10], 12, cursor + 9, content_width - 134, 18);
    place(controls[11], content_width - 100, cursor, 100, 36);
    cursor += 52;
  }
  row(12, 0, 24, content_width - 28);
  row(13, 10, 24, content_width - 28);
  row(14, 0, 18);
  row(15, 12, 36, content_width - 28);
  if (visible(controls[16])) {
    place(controls[16], 0, cursor, 100, 34);
    cursor += 34;
  }
  row(18, 12, 44, content_width - 28);
  row(19, 0, 44, content_width - 28);
  resize_shell(page, cursor + 16);
}

// Native NSIS controls remain authoritative for whether a failed worker has
// finished. In particular, failure enables Cancel, not the disabled Next button.
void refresh_actions() {
  HWND next = GetDlgItem(root, IDOK);
  HWND cancel = GetDlgItem(root, IDCANCEL);
  HWND back = GetDlgItem(root, 3);
  if (visible(back)) ShowWindow(back, SW_HIDE);
  int state = phase();
  // NSIS enables Next only after its worker completes. Keep its readiness as
  // the authority while allowing the custom titlebar to close a finished page.
  if (state == 1 && IsWindowEnabled(next)) {
    state = 3;
    SetPropW(root, L"PanitaPhase", reinterpret_cast<HANDLE>(3));
  }
  if (state == 3 || state == 4) EnableWindow(cancel, TRUE);
  const bool failed_close = state == 2 && IsWindowEnabled(cancel);
  HWND action = failed_close ? cancel : next;
  const bool show_cancel = !failed_close && state == 0;
  ShowWindow(action == cancel ? next : cancel, show_cancel ? SW_SHOWNA : SW_HIDE);
  ShowWindow(action, state == 1 ? SW_HIDE : SW_SHOWNA);
  if (failed_close) SetWindowTextW(cancel, L"Cerrar");
  else if (state == 3) {
    const bool uninstall = GetPropW(root, L"PanitaUninstall") != nullptr;
    SetWindowTextW(next, uninstall ? L"Cerrar" : L"Continuar");
  }
  HDC dc = GetDC(action);
  HGDIOBJ previous_font = SelectObject(dc, reinterpret_cast<HFONT>(SendMessageW(action, WM_GETFONT, 0, 0)));
  RECT measured{};
  const auto label = window_text(action);
  DrawTextW(dc, label.c_str(), -1, &measured, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
  SelectObject(dc, previous_font);
  ReleaseDC(action, dc);
  const int width = (std::max)(112, MulDiv(measured.right, 96, GetDpiForWindow(root)) + 56);
  place(action, shell_width - 28 - width, shell_height - 68, width, 40);
  SetWindowPos(action, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
  if (show_cancel) {
    place(cancel, shell_width - 146 - width, shell_height - 68, 108, 40);
    SetWindowPos(cancel, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
  }

  if (current_footer != action) {
    current_footer = action;
    if (failed_close) {
      SendMessageW(root, DM_SETDEFID, IDCANCEL, 0);
      if (!IsIconic(root)) SetFocus(cancel);
    }
  }
  Controls controls{};
  EnumChildWindows(root, collect_roles, reinterpret_cast<LPARAM>(&controls));
  if (controls[22]) {
    place(controls[22], shell_width - 166 - width, shell_height - 84, 100, 40);
    ShowWindow(controls[22], state == 4 ? SW_SHOWNA : SW_HIDE);
  }
  if (controls[26]) {
    HWND source = static_cast<HWND>(GetPropW(controls[26], L"PanitaProgressSource"));
    PBRANGE range{};
    SendMessageW(source, PBM_GETRANGE, FALSE, reinterpret_cast<LPARAM>(&range));
    const int position = state == 3 ? range.iHigh : static_cast<int>(SendMessageW(source, PBM_GETPOS, 0, 0));
    SendMessageW(controls[26], PBM_SETRANGE32, range.iLow, range.iHigh);
    if (SendMessageW(controls[26], PBM_GETPOS, 0, 0) != position) SendMessageW(controls[26], PBM_SETPOS, position, 0);
  }
  // The finished page starts with its details collapsed, even if they were opened while it ran.
  static bool completion_collapsed;
  if (state != 3) completion_collapsed = false;
  else if (!completion_collapsed) {
    completion_collapsed = true;
    if (details_open) {
      details_open = false;
      if (controls[21]) SetWindowTextW(controls[21], L"Mostrar detalles");
      PostMessageW(root, apply_message, 0, 0);
    }
  }
  EnableWindow(controls[3], TRUE);
  EnableWindow(controls[4], state != 1 && IsWindowEnabled(cancel));
  if (detail_source && (state == 2 || state == 3)) {
    HWND page = GetParent(detail_source);
    const bool uninstall = GetPropW(root, L"PanitaUninstall") != nullptr;
    const bool updating = GetPropW(root, L"PanitaUpdating") != nullptr;
    const wchar_t* title = state == 2 ? L"Operaci\u00f3n detenida" :
        uninstall ? L"Desinstalaci\u00f3n completada" :
        updating ? L"Actualizaci\u00f3n completada" : L"Instalaci\u00f3n completada";
    for (HWND child = GetWindow(page, GW_CHILD); child; child = GetWindow(child, GW_HWNDNEXT))
      if (GetPropW(child, L"PanitaStageTitle")) SetWindowTextW(child, title);
  }
}

HWND header_control(HWND page, const wchar_t* type, const wchar_t* text, int tag, DWORD style = 0) {
  HWND window = CreateWindowExW(0, type, text, WS_CHILD | WS_VISIBLE | style,
      0, 0, 0, 0, page, nullptr, GetModuleHandleW(nullptr), nullptr);
  SetPropW(window, L"PanitaRole", reinterpret_cast<HANDLE>(static_cast<INT_PTR>(tag)));
  return window;
}
void progress_header(HWND page, Controls& controls) {
  if (!controls[1]) {
    controls[1] = header_control(page, L"STATIC", L"", 1, SS_ICON);
    SendMessageW(controls[1], STM_SETICON, reinterpret_cast<WPARAM>(GetPropW(root, L"PanitaIcon")), 0);
    controls[2] = header_control(page, L"STATIC", L"PANITA CLIENT", 2, SS_CENTERIMAGE);
    controls[3] = header_control(page, L"BUTTON", L"Minimizar", 3, BS_PUSHBUTTON | WS_TABSTOP);
    controls[4] = header_control(page, L"BUTTON", L"Cerrar", 4, BS_PUSHBUTTON | WS_TABSTOP);
    const auto version = window_text(static_cast<HWND>(GetPropW(root, L"PanitaVersionLabel")));
    controls[5] = header_control(page, L"STATIC", version.c_str(), 5, SS_CENTERIMAGE);
    SetWindowTextW(controls[2], (L"PANITA CLIENT · " + version).c_str());

    HFONT font = CreateFontW(-MulDiv(11, GetDpiForWindow(root), 72), 0, 0, 0,
        FW_BOLD, FALSE, FALSE, FALSE, DEFAULT_CHARSET, 0, 0, CLEARTYPE_QUALITY, 0, L"Segoe UI");
    SetPropW(controls[2], L"PanitaOwnedFont", font);
    SendMessageW(controls[2], WM_SETFONT, reinterpret_cast<WPARAM>(font), 0);
  }
  position_header(controls);
}

// Mirror NSIS's own detail entries into a readonly, word-wrapped control. The
// original list remains the writer target; no log files or user data are read.
void sync_log() {
  log_sync_queued = false;
  if (!IsWindow(detail_source) || !IsWindow(wrapped_log)) return;
  std::wstring text;
  std::array<wchar_t, 32768> entry{};
  const int count = ListView_GetItemCount(detail_source);
  bool trimmed = false;
  for (int index = 0; index < count; ++index) {
    ListView_GetItemText(detail_source, index, 0, entry.data(), static_cast<int>(entry.size()));
    text.append(entry.data()).append(L"\r\n");
    while (text.size() > 1048000) {
      const size_t end = text.find(L"\r\n");
      if (end == std::wstring::npos) break;
      text.erase(0, end + 2);
      trimmed = true;
    }
  }
  if (trimmed) text.insert(0, L"Se muestran los mensajes m\u00e1s recientes.\r\n");
  if (text == window_text(wrapped_log)) return;
  const auto first = SendMessageW(wrapped_log, EM_GETFIRSTVISIBLELINE, 0, 0);
  const auto lines = SendMessageW(wrapped_log, EM_GETLINECOUNT, 0, 0);
  RECT bounds{};
  GetClientRect(wrapped_log, &bounds);
  const bool at_bottom = first + bounds.bottom / (std::max)(1, scale(18)) >= lines - 1;
  SetWindowTextW(wrapped_log, text.c_str());
  SendMessageW(wrapped_log, EM_LINESCROLL, 0,
      at_bottom ? SendMessageW(wrapped_log, EM_GETLINECOUNT, 0, 0) : first);
}
void prepare_log(Controls& controls) {
  if (!controls[17] || controls[6] || !IsWindow(detail_source)) return;
  HWND page = GetParent(detail_source);
  progress_header(page, controls);
  if (!IsWindow(wrapped_log)) {
    wrapped_log = CreateWindowExW(0, L"EDIT", L"", WS_CHILD | WS_VISIBLE | WS_TABSTOP |
        ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL,
        0, 0, 0, 0, page, nullptr, GetModuleHandleW(nullptr), nullptr);
    if (!wrapped_log) return; // Retain the original details if creation fails.
    SetPropW(wrapped_log, L"PanitaWrappedLog", reinterpret_cast<HANDLE>(1));
    SendMessageW(wrapped_log, WM_SETFONT, reinterpret_cast<WPARAM>(body_font), TRUE);
    SendMessageW(wrapped_log, EM_SETLIMITTEXT, 1048576, 0);
    SendMessageW(wrapped_log, EM_SETMARGINS, EC_LEFTMARGIN | EC_RIGHTMARGIN,
        MAKELPARAM(scale(8), scale(8)));
  }
  ShowWindow(detail_source, SW_HIDE);
  HWND progress_control = GetDlgItem(page, 1004);
  if (!controls[26]) controls[26] = header_control(page, PROGRESS_CLASSW, L"", 26, PBS_SMOOTH);
  SetPropW(controls[26], L"PanitaProgressSource", progress_control);
  ShowWindow(progress_control, SW_HIDE);
  SetWindowLongPtrW(progress_control, GWL_STYLE, GetWindowLongPtrW(progress_control, GWL_STYLE) & ~WS_BORDER);
  SetWindowLongPtrW(progress_control, GWL_EXSTYLE, GetWindowLongPtrW(progress_control, GWL_EXSTYLE) & ~WS_EX_CLIENTEDGE);
  SetWindowPos(progress_control, nullptr, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
  if (!controls[21]) {
    controls[21] = header_control(page, L"BUTTON", L"Mostrar detalles", 21, BS_PUSHBUTTON | WS_TABSTOP);
  }
  const bool warning = visible(controls[17]);
  HWND notice_frame = controls[27];
  int cursor = 140;
  if (warning) {
    cursor = layout_notice(page, notice_frame, controls[17], cursor) + 16;
  } else if (notice_frame) ShowWindow(notice_frame, SW_HIDE);
  ShowWindow(GetDlgItem(page, 1006), warning ? SW_HIDE : SW_SHOWNA);
  ShowWindow(controls[26], warning ? SW_HIDE : SW_SHOWNA);
  if (!warning) {
    place(GetDlgItem(page, 1006), 0, cursor, content_width, 36);
    place(controls[26], 0, cursor + 48, content_width, 10);
    cursor += 80;
  }
  place(controls[21], 0, cursor, 170, 34);
  place(wrapped_log, 0, cursor + 46, content_width, 120);
  ShowWindow(wrapped_log, details_open ? SW_SHOWNA : SW_HIDE);
  for (HWND child = GetWindow(page, GW_CHILD); child; child = GetWindow(child, GW_HWNDNEXT))
    if (GetPropW(child, L"PanitaStageTitle")) place(child, 0, 84, content_width, 34);
  resize_shell(page, details_open ? cursor + 182 : (std::max)(340, cursor + 50));
  sync_log();
}

bool choice_control(HWND window) {
  wchar_t type[32]{};
  GetClassNameW(window, type, 32);
  if (lstrcmpiW(type, L"Button") != 0) return false;
  const LONG_PTR kind = GetWindowLongPtrW(window, GWL_STYLE) & BS_TYPEMASK;
  return kind == BS_CHECKBOX || kind == BS_AUTOCHECKBOX ||
      kind == BS_RADIOBUTTON || kind == BS_AUTORADIOBUTTON;
}

void draw_choice(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  const UINT dpi = GetDpiForWindow(root);
  HBRUSH base = CreateSolidBrush(background);
  FillRect(dc, &bounds, base);
  DeleteObject(base);
  const int size = MulDiv(role(window) == 12 ? 16 : 18, dpi, 96);
  const bool checked = SendMessageW(window, BM_GETCHECK, 0, 0) == BST_CHECKED;
  const bool enabled = IsWindowEnabled(window) != FALSE;
  const bool hover = enabled && hovered_button == window;
  const LONG_PTR kind = GetWindowLongPtrW(window, GWL_STYLE) & BS_TYPEMASK;
  const bool radio = kind == BS_RADIOBUTTON || kind == BS_AUTORADIOBUTTON;
  const COLORREF accent = !enabled ? RGB(135,135,135) : hover ? RGB(179,255,153) : accent_color;
  RECT box{0, (bounds.bottom - size) / 2, size, (bounds.bottom + size) / 2};
  if (radio) {
    Gdiplus::Graphics graphics(dc);
    graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
    Gdiplus::SolidBrush fill(installer_ui::color(card));
    Gdiplus::Pen outline(installer_ui::color(accent), static_cast<float>(scale(1)));
    graphics.FillEllipse(&fill, box.left + 1, box.top + 1, size - 2, size - 2);
    graphics.DrawEllipse(&outline, box.left + 1, box.top + 1, size - 2, size - 2);
    if (checked) {
      Gdiplus::SolidBrush dot(installer_ui::color(accent));
      const int inset = scale(5);
      graphics.FillEllipse(&dot, box.left + inset, box.top + inset, size - inset * 2, size - inset * 2);
    }
  } else {
    installer_ui::surface(dc, box, checked ? accent : card, accent, scale(2));
    if (checked) installer_ui::icon(dc, installer_ui::Icon::check, box.left, box.top, size, background);
  }
  wchar_t text[256]{};
  GetWindowTextW(window, text, 256);
  HGDIOBJ old_font = SelectObject(dc,
      reinterpret_cast<HFONT>(SendMessageW(window, WM_GETFONT, 0, 0)));
  SetBkMode(dc, TRANSPARENT);
  SetTextColor(dc, role(window) == 12 && !hover ? RGB(186, 186, 186) : IsWindowEnabled(window) ? RGB(249, 255, 248) : RGB(135, 135, 135));
  RECT text_bounds = bounds;
  text_bounds.left = size + MulDiv(10, dpi, 96);
  RECT measured = text_bounds;
  DrawTextW(dc, text, -1, &measured, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
  text_bounds.top = (bounds.bottom - (measured.bottom - measured.top)) / 2;
  DrawTextW(dc, text, -1, &text_bounds, DT_WORDBREAK | DT_NOPREFIX);
  if (GetFocus() == window && !(SendMessageW(window, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS)) {
    installer_ui::ring(dc, bounds, RGB(228, 255, 218), scale(4), static_cast<float>(scale(2)));
  }
  SelectObject(dc, old_font);
}

struct HitTest { POINT point; bool interactive = false; };
BOOL CALLBACK interactive_at(HWND window, LPARAM data) {
  auto& hit = *reinterpret_cast<HitTest*>(data);
  for (HWND ancestor = window; ancestor && ancestor != root; ancestor = GetParent(ancestor))
    if (!visible(ancestor)) return TRUE;
  RECT bounds{};
  GetWindowRect(window, &bounds);
  if (!PtInRect(&bounds, hit.point)) return TRUE;
  wchar_t type[40]{};
  GetClassNameW(window, type, 40);
  if (lstrcmpiW(type, L"STATIC") && lstrcmpiW(type, L"#32770") && lstrcmpiW(type, PROGRESS_CLASSW)) {
    hit.interactive = true;
    return FALSE;
  }
  return TRUE;
}
bool draggable_point(LPARAM position) {
  POINT point{GET_X_LPARAM(position), GET_Y_LPARAM(position)};
  POINT client = point;
  ScreenToClient(root, &client);
  RECT bounds{};
  GetClientRect(root, &bounds);
  if (!PtInRect(&bounds, client)) return false;
  HitTest hit{point};
  EnumChildWindows(root, interactive_at, reinterpret_cast<LPARAM>(&hit));
  return !hit.interactive;
}

// Keyboard order: page content in reading order, then the footer buttons (secondary, then the
// primary action) and the window buttons last. The native order follows creation order, which
// would put the window buttons first, so Tab is handled here. Only real tab stops take part.
int tab_rank(HWND window) {
  const int tag = role(window);
  if (tag == 3 || tag == 4) return 3;
  if (window == current_footer) return 2;
  if (window == GetDlgItem(root, IDOK) || window == GetDlgItem(root, IDCANCEL) || tag == 22) return 1;
  return 0;
}
struct TabStops { std::vector<HWND> windows; };
BOOL CALLBACK collect_tab_stops(HWND window, LPARAM data) {
  if ((GetWindowLongPtrW(window, GWL_STYLE) & WS_TABSTOP) && IsWindowVisible(window) && IsWindowEnabled(window))
    reinterpret_cast<TabStops*>(data)->windows.push_back(window);
  return TRUE;
}
// True once the keyboard has moved focus on the current page. NSIS starts every page with focus on
// the default action, so the first Tab there means "start at the first control", not "continue".
bool keyboard_navigated;
bool move_focus(HWND from, bool backwards) {
  const bool page_start = !keyboard_navigated && from == current_footer;
  keyboard_navigated = true;
  TabStops stops;
  EnumChildWindows(root, collect_tab_stops, reinterpret_cast<LPARAM>(&stops));
  auto& order = stops.windows;
  if (order.size() < 2) return false;
  auto key = [](HWND window) {
    RECT bounds{};
    GetWindowRect(window, &bounds);
    const int rank = tab_rank(window);
    return std::array<LONG, 3>{rank, rank == 0 ? bounds.top : 0, bounds.left};
  };
  std::stable_sort(order.begin(), order.end(), [&](HWND a, HWND b) { return key(a) < key(b); });
  const auto found = std::find(order.begin(), order.end(), from);
  const size_t count = order.size();
  const size_t index = found == order.end() ? (backwards ? 0 : count - 1) : static_cast<size_t>(found - order.begin());
  SetFocus(order[page_start && !backwards ? 0 : backwards ? (index + count - 1) % count : (index + 1) % count]);
  // Keyboard use reveals focus cues, which Windows otherwise hides until it sees a key.
  SendMessageW(root, WM_CHANGEUISTATE, MAKEWPARAM(UIS_CLEAR, UISF_HIDEFOCUS), 0);
  return true;
}

// Tab is taken before the dialog manager sees it, for this window only, so the order above applies
// even when nothing has focus yet. Other windows of the process (such as the folder picker) and
// Ctrl+Tab or Alt+Tab keep their normal behavior.
HHOOK tab_hook;
LRESULT CALLBACK intercept_tab(int code, WPARAM wp, LPARAM lp) {
  if (code == HC_ACTION && wp == PM_REMOVE && IsWindow(root)) {
    auto& key = *reinterpret_cast<MSG*>(lp);
    if (key.message == WM_KEYDOWN && key.wParam == VK_TAB && GetAncestor(key.hwnd, GA_ROOT) == root &&
        !(GetKeyState(VK_CONTROL) & 0x8000) && !(GetKeyState(VK_MENU) & 0x8000) &&
        move_focus(GetFocus(), GetKeyState(VK_SHIFT) < 0))
      key.message = WM_NULL;
  }
  return CallNextHookEx(tab_hook, code, wp, lp);
}

void draw_button(const DRAWITEMSTRUCT& item) {
  const bool primary = item.hwndItem == (current_footer ? current_footer : GetDlgItem(root, IDOK));
  const bool disabled = (item.itemState & ODS_DISABLED) != 0;
  const bool pressed = (item.itemState & ODS_SELECTED) != 0;
  const bool hover = hovered_button == item.hwndItem && !disabled;
  const int tag = role(item.hwndItem);
  const bool chrome = tag == 3 || tag == 4;
  const bool disclosure = tag == 21;
  const bool destructive = primary && GetPropW(root, L"PanitaUninstall") && phase() == 0;
  HBRUSH base = CreateSolidBrush(chrome ? card : background);
  FillRect(item.hDC, &item.rcItem, base);
  DeleteObject(base);
  const COLORREF fill = disabled ? RGB(42, 42, 42) :
      chrome ? (hover ? (tag == 4 ? RGB(84, 39, 39) : RGB(48, 48, 48)) : card) :
      destructive ? (hover ? RGB(254, 202, 202) : RGB(252, 165, 165)) :
      primary ? (pressed ? RGB(4, 117, 0) : hover ? RGB(179, 255, 153) : accent_color) :
      (hover ? RGB(56, 56, 56) : RGB(27, 27, 27));
  installer_ui::surface(item.hDC, item.rcItem, fill,
      primary || chrome || disclosure ? fill : RGB(81, 81, 81), scale(6));
  wchar_t text[256]{};
  GetWindowTextW(item.hwndItem, text, 256);
  auto font = reinterpret_cast<HFONT>(SendMessageW(item.hwndItem, WM_GETFONT, 0, 0));
  HGDIOBJ old_font = SelectObject(item.hDC, font);
  SetBkMode(item.hDC, TRANSPARENT);
  SetTextColor(item.hDC, disabled ? RGB(135, 135, 135)
                               : primary ? background : RGB(249, 255, 248));
  RECT bounds = item.rcItem;
  if (tag == 3 || tag == 4) {
    installer_ui::icon(item.hDC, tag == 4 ? installer_ui::Icon::close : installer_ui::Icon::minimize,
        (bounds.left + bounds.right) / 2 - scale(8), (bounds.top + bounds.bottom) / 2 - scale(8),
        scale(16), disabled ? RGB(135,135,135) : RGB(249,255,248));
  } else {
    std::wstring label(text);
    label.erase(std::remove(label.begin(), label.end(), L'&'), label.end());
    if (primary || tag == 22 || item.hwndItem == GetDlgItem(root, IDCANCEL)) {
      RECT measured{};
      DrawTextW(item.hDC, label.c_str(), -1, &measured, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
      const int icon_x = (bounds.right - measured.right - scale(26)) / 2;
      const int icon_y = (bounds.top + bounds.bottom) / 2;
      const auto glyph = label == L"Cerrar" || label == L"Cancelar" ? installer_ui::Icon::close :
          destructive ? installer_ui::Icon::trash : installer_ui::Icon::arrow;
      installer_ui::icon(item.hDC, glyph, icon_x - scale(2), icon_y - scale(8), scale(16),
          disabled ? RGB(135,135,135) : primary ? background : RGB(249,255,248));
      bounds.left += scale(24);
    }
    if (disclosure) {
      installer_ui::icon(item.hDC, details_open ? installer_ui::Icon::up : installer_ui::Icon::down,
          scale(1), (bounds.bottom - scale(16)) / 2, scale(16), RGB(186,186,186));
      bounds.left += scale(24);
    }
    DrawTextW(item.hDC, label.c_str(), -1, &bounds, (disclosure ? DT_LEFT : DT_CENTER) | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  }
  if ((item.itemState & ODS_FOCUS) && !(item.itemState & ODS_NOFOCUSRECT)) {
    installer_ui::ring(item.hDC, item.rcItem, RGB(228, 255, 218), scale(6), static_cast<float>(scale(2)));
  }
  SelectObject(item.hDC, old_font);
}

void paint_surface(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  FillRect(dc, &bounds, background_brush);
  RECT header = bounds;
  POINT origin{};
  MapWindowPoints(window, root, &origin, 1);
  header.bottom = (std::max)(0, scale(72) - static_cast<int>(origin.y));
  if (header.bottom) FillRect(dc, &header, log_brush);
  HPEN pen = CreatePen(PS_SOLID, 1, RGB(48, 48, 48));
  HGDIOBJ previous = SelectObject(dc, pen);
  MoveToEx(dc, 0, scale(72) - origin.y, nullptr);
  LineTo(dc, bounds.right, scale(72) - origin.y);
  MoveToEx(dc, scale(shell_width - 116) - origin.x, scale(24) - origin.y, nullptr);
  LineTo(dc, scale(shell_width - 116) - origin.x, scale(48) - origin.y);
  SelectObject(dc, previous);
  DeleteObject(pen);
  // The frameless window gets a quiet outline that follows its rounded corners.
  if (window == root) installer_ui::ring(dc, bounds, RGB(81, 81, 81), scale(10), static_cast<float>(scale(1)));
}
void draw_badge(HDC dc, RECT bounds, const std::wstring& text) {
  FillRect(dc, &bounds, log_brush);
  HBRUSH fill = CreateSolidBrush(RGB(37, 37, 37));
  HPEN border = CreatePen(PS_SOLID, 1, RGB(65, 65, 65));
  HGDIOBJ previous_brush = SelectObject(dc, fill), previous_pen = SelectObject(dc, border);
  RoundRect(dc, bounds.left, bounds.top, bounds.right, bounds.bottom, scale(10), scale(10));
  SelectObject(dc, previous_brush); SelectObject(dc, previous_pen);
  DeleteObject(fill); DeleteObject(border);
  HGDIOBJ font = SelectObject(dc, caption_font);
  SetBkMode(dc, TRANSPARENT); SetTextColor(dc, RGB(186,186,186));
  DrawTextW(dc, text.c_str(), -1, &bounds, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  SelectObject(dc, font);
}
void paint_brand(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  FillRect(dc, &bounds, log_brush);
  SetBkMode(dc, TRANSPARENT);
  SetTextColor(dc, RGB(249, 255, 248));
  HGDIOBJ previous = SelectObject(dc, reinterpret_cast<HFONT>(SendMessageW(window, WM_GETFONT, 0, 0)));
  RECT measured{};
  DrawTextW(dc, L"PANITA", -1, &measured, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
  DrawTextW(dc, L"PANITA", -1, &bounds, DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  SelectObject(dc, body_font);
  bounds.left += measured.right + scale(6);
  DrawTextW(dc, L"CLIENT", -1, &bounds, DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  RECT client{};
  DrawTextW(dc, L"CLIENT", -1, &client, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
  bounds.left += client.right + scale(10);
  const auto version = window_text(static_cast<HWND>(GetPropW(root, L"PanitaVersionLabel")));
  SelectObject(dc, caption_font);
  SetTextColor(dc, RGB(138, 138, 138));
  DrawTextW(dc, version.c_str(), -1, &bounds, DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  SelectObject(dc, previous);
}
void paint_card(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  FillRect(dc, &bounds, background_brush);
  const bool error = role(window) == 27;
  installer_ui::surface(dc, bounds, error ? RGB(42,28,28) : card,
      error ? RGB(107,59,59) : RGB(56,56,56), scale(9));
  if (role(window) != 25)
    installer_ui::icon(dc, error ? installer_ui::Icon::error : installer_ui::Icon::info,
        scale(16), scale(17), scale(16), error ? RGB(252,165,165) : RGB(169,229,140));

}

void paint_progress(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  FillRect(dc, &bounds, background_brush);
  HBRUSH track = CreateSolidBrush(RGB(56, 56, 56));
  HGDIOBJ previous_brush = SelectObject(dc, track);
  HGDIOBJ previous_pen = SelectObject(dc, GetStockObject(NULL_PEN));
  RoundRect(dc, 0, 0, bounds.right, bounds.bottom, bounds.bottom, bounds.bottom);
  PBRANGE range{};
  SendMessageW(window, PBM_GETRANGE, FALSE, reinterpret_cast<LPARAM>(&range));
  const auto position = static_cast<int>(SendMessageW(window, PBM_GETPOS, 0, 0));
  const int width = range.iHigh > range.iLow ? MulDiv(bounds.right,
      std::clamp(position, range.iLow, range.iHigh) - range.iLow, range.iHigh - range.iLow) : 0;
  HBRUSH fill = CreateSolidBrush(accent_color);
  SelectObject(dc, fill);
  if (width > 0) RoundRect(dc, 0, 0, width, bounds.bottom, bounds.bottom, bounds.bottom);
  SelectObject(dc, previous_brush);
  SelectObject(dc, previous_pen);
  DeleteObject(fill);
  DeleteObject(track);
}

LRESULT CALLBACK presentation(HWND window, UINT message, WPARAM wp, LPARAM lp,
                              UINT_PTR id, DWORD_PTR) {
  if (window == root && message == sync_log_message) {
    sync_log();
    return 0;
  }
  if (window == root && message == apply_message) {
    apply_presentation(root);
    return 0;
  }
  if (window == root && message == WM_TIMER && wp == controls_timer) {
    refresh_actions();
    return 0;
  }
  if (window == root && message == WM_CLOSE && phase() == 1) return 0;
  if (GetDlgCtrlID(window) == 1006 && message == WM_SETTEXT && lp && phase() == 1) {
    std::wstring status(reinterpret_cast<const wchar_t*>(lp));
    std::transform(status.begin(), status.end(), status.begin(), [](wchar_t c) { return static_cast<wchar_t>(std::towlower(c)); });
    const wchar_t* text = GetPropW(root, L"PanitaUpdating") ? L"Actualizando archivos..." : L"Instalando archivos...";
    if (GetPropW(root, L"PanitaUninstall")) text = L"Eliminando archivos...";
    else if (status.find(L"desinstalador") != std::wstring::npos || status.find(L"uninstaller") != std::wstring::npos) text = L"Creando desinstalador...";
    else if (status.find(L"acceso") != std::wstring::npos || status.find(L"shortcut") != std::wstring::npos) text = L"Creando accesos directos...";
    else if (status.find(L"extra") != std::wstring::npos || status.find(L"directorio") != std::wstring::npos) text = L"Extrayendo archivos...";
    return DefSubclassProc(window, message, wp, reinterpret_cast<LPARAM>(text));
  }
  wchar_t window_class[32]{};
  GetClassNameW(window, window_class, 32);
  if (message == WM_MOUSEMOVE && lstrcmpiW(window_class, L"Button") == 0) {
    if (hovered_button != window) {
      if (IsWindow(hovered_button)) InvalidateRect(hovered_button, nullptr, FALSE);
      hovered_button = window;
      InvalidateRect(window, nullptr, FALSE);
    }
    TRACKMOUSEEVENT tracking{sizeof(tracking), TME_LEAVE, window, 0};
    TrackMouseEvent(&tracking);
  }
  if (message == WM_SETCURSOR && lstrcmpiW(window_class, L"Button") == 0 && IsWindowEnabled(window)) {
    SetCursor(LoadCursorW(nullptr, IDC_HAND));
    return TRUE;
  }
  if (message == WM_MOUSELEAVE && hovered_button == window) {
    hovered_button = nullptr;
    InvalidateRect(window, nullptr, FALSE);
  }
  if (message == WM_ERASEBKGND && (window == root || lstrcmpiW(window_class, L"#32770") == 0)) {
    paint_surface(window, reinterpret_cast<HDC>(wp));
    return TRUE;
  }
  // Paint controls directly as well as handling WM_DRAWITEM. Native NSIS dialog
  // procedures can consume owner-draw notifications during page transitions.
  if (lstrcmpiW(window_class, L"Button") == 0 && !choice_control(window) &&
      (message == WM_PAINT || message == WM_PRINTCLIENT)) {
    PAINTSTRUCT paint{};
    DRAWITEMSTRUCT item{};
    item.CtlType = ODT_BUTTON;
    item.hwndItem = window;
    item.hDC = message == WM_PAINT ? BeginPaint(window, &paint) : reinterpret_cast<HDC>(wp);
    GetClientRect(window, &item.rcItem);
    const auto state = SendMessageW(window, BM_GETSTATE, 0, 0);
    item.itemState = (!IsWindowEnabled(window) ? ODS_DISABLED : 0) |
        (state & BST_PUSHED ? ODS_SELECTED : 0) | (GetFocus() == window ? ODS_FOCUS : 0) |
        (SendMessageW(window, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS ? ODS_NOFOCUSRECT : 0);
    draw_button(item);
    if (message == WM_PAINT) EndPaint(window, &paint);
    return 0;
  }
  const int text_role = role(window);
  if ((text_role == 6 || text_role == 7 || text_role == 9 || text_role == 14 ||
      text_role == 17 || text_role == 24 || GetPropW(window, L"PanitaStageTitle")) &&
      (message == WM_PAINT || message == WM_PRINTCLIENT)) {
    PAINTSTRUCT paint{};
    HDC dc = message == WM_PAINT ? BeginPaint(window, &paint) : reinterpret_cast<HDC>(wp);
    RECT bounds{}; GetClientRect(window, &bounds);
    const bool error = text_role == 17 || GetPropW(window, L"PanitaError");
    HBRUSH brush = CreateSolidBrush(error ? RGB(42,28,28) : text_role == 9 ? card : background);
    FillRect(dc, &bounds, brush); DeleteObject(brush);
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, error ? RGB(252,165,165) : text_role == 6 || GetPropW(window, L"PanitaStageTitle") ?
        RGB(249,255,248) : GetPropW(window, L"PanitaSubtle") ? RGB(138,138,138) : RGB(186,186,186));
    HGDIOBJ old = SelectObject(dc, reinterpret_cast<HFONT>(SendMessageW(window, WM_GETFONT, 0, 0)));
    const auto text = window_text(window);
    DrawTextW(dc, text.c_str(), -1, &bounds, DT_WORDBREAK | DT_NOPREFIX);
    SelectObject(dc, old);
    if (message == WM_PAINT) EndPaint(window, &paint);
    return 0;
  }
  const bool progress = role(window) == 26;
  if ((role(window) == 2 || role(window) == 5 || role(window) == 8 || role(window) == 25 || role(window) == 27 || progress) && (message == WM_PAINT || message == WM_PRINTCLIENT)) {
    PAINTSTRUCT paint{};
    HDC dc = message == WM_PAINT ? BeginPaint(window, &paint) : reinterpret_cast<HDC>(wp);
    if (progress) paint_progress(window, dc);
    else if (role(window) == 2) paint_brand(window, dc);
    else if (role(window) == 5) {
      RECT bounds{}; GetClientRect(window, &bounds); draw_badge(dc, bounds, window_text(window));
    }

    else paint_card(window, dc);
    if (message == WM_PAINT) EndPaint(window, &paint);
    return 0;
  }
  if (progress && (message == PBM_SETPOS || message == PBM_DELTAPOS ||
      message == PBM_SETSTEP || message == PBM_STEPIT || message == PBM_SETRANGE || message == PBM_SETRANGE32)) {
    const LRESULT result = DefSubclassProc(window, message, wp, lp);
    InvalidateRect(window, nullptr, FALSE);
    return result;
  }
  if (message == WM_COMMAND && HIWORD(wp) == BN_CLICKED && lp) {
    const int tag = role(reinterpret_cast<HWND>(lp));
    if (tag == 22 && phase() == 4) { SendMessageW(root, WM_COMMAND, IDCANCEL, 0); return 0; }
    if (tag == 21) {
      details_open = !details_open;
      SetWindowTextW(reinterpret_cast<HWND>(lp), details_open ? L"Ocultar detalles" : L"Mostrar detalles");
      apply_presentation(root);
      return 0;
    }
    if (tag == 3) { ShowWindow(root, SW_MINIMIZE); return 0; }
    if (tag == 4) {
      if (phase() != 1 && IsWindowEnabled(GetDlgItem(root, IDCANCEL)))
        SendMessageW(root, WM_COMMAND, IDCANCEL, 0);
      return 0;
    }
  }
  // NSIS changes these controls after a section finishes. Never let its old
  // Back/Cancel positions reappear over the log or the custom completion page.
  if (message == WM_WINDOWPOSCHANGING && lp &&
      (window == GetDlgItem(root, 3) || (window == GetDlgItem(root, IDCANCEL) && phase() == 1))) {
    auto& position = *reinterpret_cast<WINDOWPOS*>(lp);
    position.flags = (position.flags & ~SWP_SHOWWINDOW) | SWP_HIDEWINDOW;
    if (window == GetDlgItem(root, 3)) {
      position.x = position.y = -10000;
      position.cx = position.cy = 0;
      position.flags &= ~(SWP_NOMOVE | SWP_NOSIZE);
    }
  }
  if (message == WM_WINDOWPOSCHANGED && window == GetDlgItem(root, 3) && visible(window)) {
    ShowWindow(window, SW_HIDE);
  }
  // The folder field is read-only, but answer both color requests so its text can never sit on a
  // different surface than the rounded field behind it.
  if ((message == WM_CTLCOLORSTATIC || message == WM_CTLCOLOREDIT) && role(reinterpret_cast<HWND>(lp)) == 10) {
    SetTextColor(reinterpret_cast<HDC>(wp), RGB(249,255,248));
    SetBkColor(reinterpret_cast<HDC>(wp), card);
    return reinterpret_cast<LRESULT>(log_brush);
  }
  if (message == WM_CTLCOLORSTATIC) {
    const int tag = role(reinterpret_cast<HWND>(lp));
    if (tag >= 1 && tag <= 5) {
      SetTextColor(reinterpret_cast<HDC>(wp), tag == 5 ? RGB(186, 186, 186) : RGB(249, 255, 248));
      SetBkColor(reinterpret_cast<HDC>(wp), tag == 5 ? background : card);
      return reinterpret_cast<LRESULT>(tag == 5 ? background_brush : log_brush);
    }
  }
  if (message == WM_CTLCOLORSTATIC && GetPropW(reinterpret_cast<HWND>(lp), L"PanitaWrappedLog")) {
    HDC dc = reinterpret_cast<HDC>(wp);
    SetTextColor(dc, RGB(249, 255, 248));
    SetBkColor(dc, card);
    return reinterpret_cast<LRESULT>(log_brush);
  }
  if (role(window) == 1 && message == WM_PAINT) {
    PAINTSTRUCT paint{};
    HDC dc = BeginPaint(window, &paint);
    draw_logo(window, dc);
    EndPaint(window, &paint);
    return 0;
  }
  if (role(window) == 1 && message == WM_PRINTCLIENT) {
    draw_logo(window, reinterpret_cast<HDC>(wp));
    return 0;
  }
  // Keep native checkbox/radio state, grouping, keyboard and accessibility.
  // Only replace painting, including wrapped text and keyboard focus.
  if (choice_control(window)) {
    if (message == WM_PAINT) {
      PAINTSTRUCT paint{};
      HDC dc = BeginPaint(window, &paint);
      draw_choice(window, dc);
      EndPaint(window, &paint);
      return 0;
    }
    if (message == WM_PRINTCLIENT) {
      draw_choice(window, reinterpret_cast<HDC>(wp));
      return 0;
    }
    if (message == WM_ERASEBKGND) return TRUE;
    if (message == BM_SETCHECK || message == WM_SETFOCUS || message == WM_KILLFOCUS ||
        message == WM_UPDATEUISTATE || message == WM_ENABLE) {
      const LRESULT result = DefSubclassProc(window, message, wp, lp);
      InvalidateRect(window, nullptr, TRUE);
      return result;
    }
  }
  if (message == BM_SETSTYLE &&
      (GetWindowLongPtrW(window, GWL_STYLE) & BS_TYPEMASK) == BS_OWNERDRAW) {
    return DefSubclassProc(window, message, (wp & ~BS_TYPEMASK) | BS_OWNERDRAW, lp);
  }
  // Push buttons repaint whenever focus or the keyboard-cue state changes, so the focus ring
  // appears on Tab and disappears when focus leaves.
  if ((message == WM_SETFOCUS || message == WM_KILLFOCUS || message == WM_UPDATEUISTATE) &&
      lstrcmpiW(window_class, L"Button") == 0 && !choice_control(window)) {
    const LRESULT result = DefSubclassProc(window, message, wp, lp);
    InvalidateRect(window, nullptr, FALSE);
    return result;
  }
  // A read-only log must not show its text selected when Tab reaches it.
  if (window == wrapped_log && message == WM_SETFOCUS) {
    const LRESULT result = DefSubclassProc(window, message, wp, lp);
    SendMessageW(window, EM_SETSEL, static_cast<WPARAM>(-1), 0);
    return result;
  }
  if (message == WM_GETDLGCODE && lstrcmpiW(window_class, L"Button") == 0 &&
      (GetWindowLongPtrW(window, GWL_STYLE) & BS_TYPEMASK) == BS_OWNERDRAW) {
    return DefSubclassProc(window, message, wp, lp) | DLGC_BUTTON |
        (window == current_footer ? DLGC_DEFPUSHBUTTON : DLGC_UNDEFPUSHBUTTON);
  }
  if (message == WM_NCHITTEST && draggable_point(lp)) {
    wchar_t type[32]{};
    GetClassNameW(window, type, 32);
    // Interactive header buttons retain normal mouse/keyboard behavior.
    if (lstrcmpiW(type, L"Button") != 0)
      return window == root ? HTCAPTION : HTTRANSPARENT;
  }
  if (message == WM_DRAWITEM && lp) {
    const auto& item = *reinterpret_cast<DRAWITEMSTRUCT*>(lp);
    if (item.CtlType == ODT_BUTTON) {
      draw_button(item);
      return TRUE;
    }
  }
  if (message == WM_NCDESTROY) {
    RemoveWindowSubclass(window, presentation, id);
    if (GetPropW(window, L"PanitaOwnedFont")) DeleteObject(GetPropW(window, L"PanitaOwnedFont"));
    if (window == detail_source) { detail_source = nullptr; wrapped_log = nullptr; log_sync_queued = false; }
    if (window == root) {
      delete brand_image; brand_image = nullptr;
      if (graphics_token) Gdiplus::GdiplusShutdown(graphics_token);
      graphics_token = 0;
      DeleteObject(body_font);
      DeleteObject(caption_font);
      DeleteObject(option_font);
      body_font = caption_font = option_font = nullptr;
      DeleteObject(log_brush);
      log_brush = nullptr;
      DeleteObject(background_brush);
      background_brush = nullptr;
      if (IsWindow(tooltips)) DestroyWindow(tooltips);
      tooltips = nullptr;
      current_footer = nullptr;
      hovered_button = nullptr;
      details_open = false;
      if (tab_hook) UnhookWindowsHookEx(tab_hook);
      tab_hook = nullptr;
    }
  }
  const LRESULT result = DefSubclassProc(window, message, wp, lp);
  if (window == detail_source && !log_sync_queued &&
      (message == LVM_INSERTITEMW || message == LVM_INSERTITEMA ||
       message == LVM_SETITEMTEXTW || message == LVM_SETITEMTEXTA ||
       message == LVM_SETITEMW || message == LVM_SETITEMA ||
       message == LVM_DELETEITEM || message == LVM_DELETEALLITEMS)) {
    log_sync_queued = true;
    PostMessageW(root, sync_log_message, 0, 0);
  }
  return result;
}

BOOL CALLBACK style_child(HWND window, LPARAM) {
  SetWindowLongPtrW(window, GWL_STYLE, GetWindowLongPtrW(window, GWL_STYLE) | WS_CLIPSIBLINGS);
  SetWindowSubclass(window, presentation, 1, 0);
  wchar_t type[32]{};
  GetClassNameW(window, type, 32);
  if (lstrcmpiW(type, L"#32770") == 0)
    SetWindowLongPtrW(window, GWL_STYLE, GetWindowLongPtrW(window, GWL_STYLE) | WS_CLIPCHILDREN);
  if (lstrcmpiW(type, WC_LISTVIEWW) == 0 && GetDlgCtrlID(window) == 1016) detail_source = window;
  const int tag = role(window);
  if ((tag == 3 || tag == 4) && !GetPropW(window, L"PanitaTooltip")) {
    if (!IsWindow(tooltips)) tooltips = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, nullptr,
        WS_POPUP | TTS_ALWAYSTIP, 0, 0, 0, 0, root, nullptr, GetModuleHandleW(nullptr), nullptr);
    TTTOOLINFOW info{};
    info.cbSize = sizeof(info);
    info.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
    info.hwnd = root;
    info.uId = reinterpret_cast<UINT_PTR>(window);
    info.lpszText = const_cast<wchar_t*>(tag == 3 ? L"Minimizar" : L"Cerrar");
    if (SendMessageW(tooltips, TTM_ADDTOOLW, 0, reinterpret_cast<LPARAM>(&info)))
      SetPropW(window, L"PanitaTooltip", reinterpret_cast<HANDLE>(1));
  }
  LOGFONTW font{};
  const auto current_font = reinterpret_cast<HFONT>(SendMessageW(window, WM_GETFONT, 0, 0));
  GetObjectW(current_font, sizeof(font), &font);
  const int control_id = GetDlgCtrlID(window);
  const bool footer_button = control_id == IDOK || control_id == IDCANCEL || tag == 22;
  if (tag == 12) {
    SendMessageW(window, WM_SETFONT, reinterpret_cast<WPARAM>(option_font), TRUE);
  } else if (footer_button) {
    // 7. The footer buttons share one font size.
    SendMessageW(window, WM_SETFONT, reinterpret_cast<WPARAM>(body_font), TRUE);
  } else if (tag == 5) {
    SetWindowLongPtrW(window, GWL_STYLE, GetWindowLongPtrW(window, GWL_STYLE) | SS_CENTERIMAGE);
    SendMessageW(window, WM_SETFONT, reinterpret_cast<WPARAM>(caption_font), TRUE);
  } else if (font.lfHeight >= -MulDiv(14, GetDpiForWindow(root), 96)) {
    RECT bounds{};
    GetWindowRect(window, &bounds);
    const bool compact_button = lstrcmpiW(type, L"Button") == 0 && !choice_control(window) &&
        bounds.bottom - bounds.top <= MulDiv(32, GetDpiForWindow(root), 96);
    SendMessageW(window, WM_SETFONT,
        reinterpret_cast<WPARAM>(compact_button ? caption_font : body_font), TRUE);
  }
  if (lstrcmpiW(type, L"Button") == 0) {
    LONG_PTR style = GetWindowLongPtrW(window, GWL_STYLE);
    const LONG_PTR kind = style & BS_TYPEMASK;
    if (kind == BS_PUSHBUTTON || kind == BS_DEFPUSHBUTTON) {
      // Preserve a real BUTTON control: tab navigation, Space, Enter and
      // accessibility remain provided by Windows rather than simulated labels.
      SendMessageW(window, BM_SETSTYLE, BS_OWNERDRAW, TRUE);
      InvalidateRect(window, nullptr, TRUE);
    }
  }
  return TRUE;
}

void apply_presentation(HWND parent) {
  if (!IsWindow(parent)) return;
  root = parent;
  SetWindowLongPtrW(root, GWL_STYLE, GetWindowLongPtrW(root, GWL_STYLE) | WS_CLIPCHILDREN);
  if (!graphics_token) {
    Gdiplus::GdiplusStartupInput input;
    Gdiplus::GdiplusStartup(&graphics_token, &input, nullptr);
  }
  if (!brand_image) {
    const auto path = window_text(static_cast<HWND>(GetPropW(root, L"PanitaBrandImagePath")));
    if (!path.empty()) {
      auto* loaded = Gdiplus::Image::FromFile(path.c_str());
      if (loaded && loaded->GetLastStatus() == Gdiplus::Ok) brand_image = loaded;
      else delete loaded;
    }
  }
  if (!body_font) {
    const UINT dpi = GetDpiForWindow(root);
    body_font = CreateFontW(-MulDiv(11, dpi, 72), 0, 0, 0, FW_NORMAL, FALSE,
        FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY, DEFAULT_PITCH, L"Segoe UI");
    caption_font = CreateFontW(-MulDiv(9, dpi, 72), 0, 0, 0, FW_NORMAL, FALSE,
        FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY, DEFAULT_PITCH, L"Segoe UI");
    option_font = CreateFontW(-MulDiv(10, dpi, 72), 0, 0, 0, FW_NORMAL, FALSE,
        FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY, DEFAULT_PITCH, L"Segoe UI");
  }
  if (!log_brush) log_brush = CreateSolidBrush(card);
  if (!background_brush) background_brush = CreateSolidBrush(background);
  SetWindowSubclass(root, presentation, 1, 0);
  keyboard_navigated = false; // Every page starts again at its first control.
  if (!tab_hook) tab_hook = SetWindowsHookExW(WH_GETMESSAGE, intercept_tab, nullptr, GetCurrentThreadId());
  EnumChildWindows(root, style_child, 0);
  Controls controls{};
  EnumChildWindows(root, collect_roles, reinterpret_cast<LPARAM>(&controls));
  compact_page(controls);
  prepare_log(controls);
  // Progress header controls are created after the first enumeration.
  EnumChildWindows(root, style_child, 0);
  refresh_actions();
  SetTimer(root, controls_timer, 100, nullptr);
  RedrawWindow(root, nullptr, nullptr, RDW_INVALIDATE | RDW_ALLCHILDREN);
}
}

extern "C" __declspec(dllexport) void __cdecl Apply(HWND parent, int, wchar_t*, void*, void*) {
  if (!IsWindow(parent)) return;
  if (GetWindowThreadProcessId(parent, nullptr) != GetCurrentThreadId()) {
    PostMessageW(parent, apply_message, 0, 0);
    return;
  }
  apply_presentation(parent);
}
