// Presentation-only NSIS plugin. No installation, filesystem or account logic.
// The parameter-free export follows NSIS's Unicode plugin ABI. /NOUNLOAD keeps
// the subclass procedures resident until the installer process exits.
#define UNICODE
#define _UNICODE
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <windowsx.h>
#include <commctrl.h>
#include <algorithm>
#include <array>
#include <string>

namespace {
constexpr COLORREF background = RGB(20, 20, 20);
constexpr COLORREF card = RGB(27, 27, 27);
constexpr COLORREF accent_color = RGB(84, 166, 54);
HWND root;
HFONT body_font;
HFONT caption_font;
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
int shell_height = 504;
bool details_open;
using Controls = std::array<HWND, 22>;
void apply_presentation(HWND parent);
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
  const auto icon = reinterpret_cast<HICON>(SendMessageW(window, STM_GETICON, 0, 0));
  DrawIconEx(dc, 0, 0, icon, bounds.right, bounds.bottom, 0, nullptr, DI_NORMAL);
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
  shell_height = (std::max)(504, content_bottom + 100);
  SetWindowPos(root, nullptr, 0, 0, scale(560), scale(shell_height),
      SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
  place(page, 28, 16, 504, shell_height - 88);
  HRGN rounded = CreateRoundRectRgn(0, 0, scale(560) + 1, scale(shell_height) + 1, scale(20), scale(20));
  if (!SetWindowRgn(root, rounded, TRUE)) DeleteObject(rounded);
}
void position_header(const Controls& controls) {
  place(controls[1], 0, 0, 40, 40);
  place(controls[2], 52, 0, 270, 40);
  place(controls[3], 432, 3, 34, 34);
  place(controls[4], 470, 3, 34, 34);
}
void compact_page(const Controls& controls) {
  if (!controls[6] || !controls[7]) return;
  HWND page = GetParent(controls[6]);
  position_header(controls);
  int cursor = 84;
  auto row = [&](int id, int gap, int minimum, int width = 504) {
    HWND control = controls[static_cast<size_t>(id)];
    if (!visible(control)) return;
    const int height = (std::max)(minimum, text_height(control, width));
    place(control, 0, cursor, 504, height);
    cursor += height + gap;
  };
  row(6, 12, 34);
  row(7, 22, 18);
  if (controls[8] && controls[9]) {
    const int height = text_height(controls[9], 472);
    place(controls[8], 0, cursor, 504, height + 32);
    place(controls[9], 16, cursor + 16, 472, height);
    cursor += height + 50;
  }
  if (visible(controls[10])) {
    place(controls[10], 0, cursor, 504, 24);
    cursor += 32;
  }
  if (visible(controls[11])) {
    place(controls[11], 0, cursor, 160, 34);
    cursor += 46;
  }
  row(12, 0, 24, 476);
  row(13, 10, 24, 476);
  row(14, 0, 18);
  row(15, 12, 36, 476);
  if (visible(controls[16])) {
    place(controls[16], 0, cursor, 100, 34);
    cursor += 34;
  }
  row(18, 12, 44, 476);
  row(19, 0, 44, 476);
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
  const bool show_cancel = !failed_close && (state == 0 || state == 4);
  ShowWindow(action == cancel ? next : cancel, show_cancel ? SW_SHOWNA : SW_HIDE);
  ShowWindow(action, SW_SHOWNA);
  HDC dc = GetDC(action);
  HGDIOBJ previous_font = SelectObject(dc, reinterpret_cast<HFONT>(SendMessageW(action, WM_GETFONT, 0, 0)));
  RECT measured{};
  const auto label = window_text(action);
  DrawTextW(dc, label.c_str(), -1, &measured, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
  SelectObject(dc, previous_font);
  ReleaseDC(action, dc);
  const int width = (std::max)(112, MulDiv(measured.right, 96, GetDpiForWindow(root)) + 56);
  place(action, 532 - width, shell_height - 68, width, 40);
  if (show_cancel) place(cancel, 422 - width, shell_height - 68, 100, 40);
  HWND version = static_cast<HWND>(GetPropW(root, L"PanitaVersionLabel"));
  place(version, 28, shell_height - 68, 120, 40);
  ShowWindow(version, SW_SHOWNA);
  if (failed_close) SetWindowTextW(cancel, L"Cerrar");
  else if (state == 3) {
    const bool uninstall = GetPropW(root, L"PanitaUninstall") != nullptr;
    SetWindowTextW(next, uninstall ? L"Cerrar" : L"Continuar");
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
  EnableWindow(controls[3], TRUE);
  EnableWindow(controls[4], state != 1 && IsWindowEnabled(cancel));
  if (detail_source && (state == 2 || state == 3)) {
    HWND page = GetParent(detail_source);
    const bool uninstall = GetPropW(root, L"PanitaUninstall") != nullptr;
    const wchar_t* title = state == 2 ? L"Operaci\u00f3n detenida" :
        uninstall ? L"Desinstalaci\u00f3n completada" : L"Instalaci\u00f3n completada";
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
  if (!controls[21]) {
    controls[21] = header_control(page, L"BUTTON", L"Ver detalles", 21, BS_PUSHBUTTON | WS_TABSTOP);
  }
  const bool warning = visible(controls[17]);
  place(controls[17], 0, 130, 504, 92);
  place(GetDlgItem(page, 1006), 0, 140, 504, 36);
  place(GetDlgItem(page, 1004), 0, 188, 504, 8);
  place(controls[21], 0, warning ? 236 : 220, 140, 34);
  place(wrapped_log, 0, warning ? 282 : 266, 504, 120);
  ShowWindow(wrapped_log, details_open ? SW_SHOWNA : SW_HIDE);
  for (HWND child = GetWindow(page, GW_CHILD); child; child = GetWindow(child, GW_HWNDNEXT))
    if (GetPropW(child, L"PanitaStageTitle")) place(child, 0, 84, 504, 34);
  resize_shell(page, details_open ? (warning ? 418 : 402) : 340);
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
  const int size = MulDiv(18, dpi, 96);
  const bool checked = SendMessageW(window, BM_GETCHECK, 0, 0) == BST_CHECKED;
  const bool enabled = IsWindowEnabled(window) != FALSE;
  const LONG_PTR kind = GetWindowLongPtrW(window, GWL_STYLE) & BS_TYPEMASK;
  const bool radio = kind == BS_RADIOBUTTON || kind == BS_AUTORADIOBUTTON;
  const COLORREF accent = enabled ? accent_color : RGB(135, 135, 135);
  RECT box{0, (bounds.bottom - size) / 2, size, (bounds.bottom + size) / 2};
  HBRUSH fill = CreateSolidBrush(checked && !radio ? accent : card);
  HPEN outline = CreatePen(PS_SOLID, (std::max)(1, scale(1)), accent);
  HGDIOBJ old_brush = SelectObject(dc, fill);
  HGDIOBJ old_outline = SelectObject(dc, outline);
  if (radio) {
    Ellipse(dc, box.left, box.top, box.right, box.bottom);
    if (checked) {
      HBRUSH dot = CreateSolidBrush(accent);
      SelectObject(dc, dot);
      const int inset = scale(5);
      Ellipse(dc, box.left + inset, box.top + inset, box.right - inset, box.bottom - inset);
      SelectObject(dc, fill);
      DeleteObject(dot);
    }
  } else {
    Rectangle(dc, box.left, box.top, box.right, box.bottom);
  }
  SelectObject(dc, old_brush);
  SelectObject(dc, old_outline);
  DeleteObject(fill);
  DeleteObject(outline);
  if (checked && !radio) {
    HPEN pen = CreatePen(PS_SOLID, MulDiv(2, dpi, 96), background);
    HGDIOBJ old_pen = SelectObject(dc, pen);
    MoveToEx(dc, size / 4, box.top + size / 2, nullptr);
    LineTo(dc, size * 4 / 9, box.top + size * 3 / 4);
    LineTo(dc, size * 4 / 5, box.top + size / 4);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
  }
  wchar_t text[256]{};
  GetWindowTextW(window, text, 256);
  HGDIOBJ old_font = SelectObject(dc,
      reinterpret_cast<HFONT>(SendMessageW(window, WM_GETFONT, 0, 0)));
  SetBkMode(dc, TRANSPARENT);
  SetTextColor(dc, IsWindowEnabled(window) ? RGB(249, 255, 248) : RGB(135, 135, 135));
  RECT text_bounds = bounds;
  text_bounds.left = size + MulDiv(10, dpi, 96);
  RECT measured = text_bounds;
  DrawTextW(dc, text, -1, &measured, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
  text_bounds.top = (bounds.bottom - (measured.bottom - measured.top)) / 2;
  DrawTextW(dc, text, -1, &text_bounds, DT_WORDBREAK | DT_NOPREFIX);
  if (GetFocus() == window && !(SendMessageW(window, WM_QUERYUISTATE, 0, 0) & UISF_HIDEFOCUS)) {
    InflateRect(&bounds, -1, -1);
    DrawFocusRect(dc, &bounds);
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

void draw_button(const DRAWITEMSTRUCT& item) {
  const bool primary = item.hwndItem == (current_footer ? current_footer : GetDlgItem(root, IDOK));
  const bool disabled = (item.itemState & ODS_DISABLED) != 0;
  const bool pressed = (item.itemState & ODS_SELECTED) != 0;
  const bool hover = hovered_button == item.hwndItem && !disabled;
  const int tag = role(item.hwndItem);
  const bool chrome = tag == 3 || tag == 4;
  const bool destructive = primary && GetPropW(root, L"PanitaUninstall") && phase() == 0;
  HBRUSH base = CreateSolidBrush(chrome ? card : background);
  FillRect(item.hDC, &item.rcItem, base);
  DeleteObject(base);
  const COLORREF fill = disabled ? RGB(42, 42, 42) :
      chrome ? (hover ? (tag == 4 ? RGB(84, 39, 39) : RGB(48, 48, 48)) : card) :
      destructive ? (hover ? RGB(254, 202, 202) : RGB(252, 165, 165)) :
      primary ? (pressed ? RGB(4, 117, 0) : hover ? RGB(101, 182, 71) : accent_color) :
      (hover ? RGB(56, 56, 56) : RGB(27, 27, 27));
  HBRUSH brush = CreateSolidBrush(fill);
  HPEN pen = CreatePen(PS_SOLID, 1, primary || chrome ? fill : RGB(81, 81, 81));
  HGDIOBJ old_brush = SelectObject(item.hDC, brush);
  HGDIOBJ old_pen = SelectObject(item.hDC, pen);
  const int radius = MulDiv(10, GetDpiForWindow(root), 96);
  RoundRect(item.hDC, item.rcItem.left, item.rcItem.top, item.rcItem.right,
            item.rcItem.bottom, radius, radius);
  SelectObject(item.hDC, old_pen);
  SelectObject(item.hDC, old_brush);
  DeleteObject(pen);
  DeleteObject(brush);
  wchar_t text[256]{};
  GetWindowTextW(item.hwndItem, text, 256);
  auto font = reinterpret_cast<HFONT>(SendMessageW(item.hwndItem, WM_GETFONT, 0, 0));
  HGDIOBJ old_font = SelectObject(item.hDC, font);
  SetBkMode(item.hDC, TRANSPARENT);
  SetTextColor(item.hDC, disabled ? RGB(135, 135, 135)
                               : primary ? background : RGB(249, 255, 248));
  RECT bounds = item.rcItem;
  if (tag == 3 || tag == 4) {
    const int center_x = (bounds.left + bounds.right) / 2;
    const int center_y = (bounds.top + bounds.bottom) / 2;
    const int half = scale(5);
    HPEN icon_pen = CreatePen(PS_SOLID, (std::max)(1, scale(1)),
        disabled ? RGB(135, 135, 135) : RGB(249, 255, 248));
    HGDIOBJ previous_pen = SelectObject(item.hDC, icon_pen);
    MoveToEx(item.hDC, center_x - half, center_y - (tag == 4 ? half : 0), nullptr);
    LineTo(item.hDC, center_x + half, center_y + (tag == 4 ? half : 0));
    if (tag == 4) {
      MoveToEx(item.hDC, center_x - half, center_y + half, nullptr);
      LineTo(item.hDC, center_x + half, center_y - half);
    }
    SelectObject(item.hDC, previous_pen);
    DeleteObject(icon_pen);
  } else {
    std::wstring label(text);
    label.erase(std::remove(label.begin(), label.end(), L'&'), label.end());
    if (primary || item.hwndItem == GetDlgItem(root, IDCANCEL)) {
      RECT measured{};
      DrawTextW(item.hDC, label.c_str(), -1, &measured, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
      const int icon_x = (bounds.right - measured.right - scale(26)) / 2;
      const int icon_y = (bounds.top + bounds.bottom) / 2;
      HPEN icon_pen = CreatePen(PS_SOLID, (std::max)(1, scale(2)),
          disabled ? RGB(135, 135, 135) : primary ? background : RGB(249, 255, 248));
      HGDIOBJ previous = SelectObject(item.hDC, icon_pen);
      const bool close = label == L"Cerrar" || label == L"Cancelar";
      if (close) {
        MoveToEx(item.hDC, icon_x, icon_y - scale(5), nullptr);
        LineTo(item.hDC, icon_x + scale(10), icon_y + scale(5));
        MoveToEx(item.hDC, icon_x, icon_y + scale(5), nullptr);
        LineTo(item.hDC, icon_x + scale(10), icon_y - scale(5));
      } else if (destructive) {
        MoveToEx(item.hDC, icon_x - scale(1), icon_y - scale(4), nullptr);
        LineTo(item.hDC, icon_x + scale(11), icon_y - scale(4));
        MoveToEx(item.hDC, icon_x + scale(2), icon_y - scale(4), nullptr);
        LineTo(item.hDC, icon_x + scale(2), icon_y + scale(6));
        LineTo(item.hDC, icon_x + scale(8), icon_y + scale(6));
        LineTo(item.hDC, icon_x + scale(8), icon_y - scale(4));
        MoveToEx(item.hDC, icon_x + scale(3), icon_y - scale(7), nullptr);
        LineTo(item.hDC, icon_x + scale(7), icon_y - scale(7));
      } else {
        MoveToEx(item.hDC, icon_x, icon_y, nullptr);
        LineTo(item.hDC, icon_x + scale(12), icon_y);
        MoveToEx(item.hDC, icon_x + scale(7), icon_y - scale(5), nullptr);
        LineTo(item.hDC, icon_x + scale(12), icon_y);
        LineTo(item.hDC, icon_x + scale(7), icon_y + scale(5));
      }
      SelectObject(item.hDC, previous);
      DeleteObject(icon_pen);
      bounds.left += scale(24);
    }
    DrawTextW(item.hDC, label.c_str(), -1, &bounds, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  }
  if ((item.itemState & ODS_FOCUS) && !(item.itemState & ODS_NOFOCUSRECT)) {
    InflateRect(&bounds, -4, -4);
    DrawFocusRect(item.hDC, &bounds);
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
  MoveToEx(dc, scale(446) - origin.x, scale(24) - origin.y, nullptr);
  LineTo(dc, scale(446) - origin.x, scale(48) - origin.y);
  SelectObject(dc, previous);
  DeleteObject(pen);
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
  SelectObject(dc, previous);
}
void paint_card(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  FillRect(dc, &bounds, background_brush);
  HPEN pen = CreatePen(PS_SOLID, 1, RGB(56, 56, 56));
  HGDIOBJ previous_pen = SelectObject(dc, pen);
  HGDIOBJ previous_brush = SelectObject(dc, log_brush);
  RoundRect(dc, 0, 0, bounds.right, bounds.bottom, scale(18), scale(18));
  SelectObject(dc, previous_brush);
  SelectObject(dc, previous_pen);
  DeleteObject(pen);
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
  if (message == WM_MOUSELEAVE && hovered_button == window) {
    hovered_button = nullptr;
    InvalidateRect(window, nullptr, FALSE);
  }
  if (message == WM_ERASEBKGND && (window == root || lstrcmpiW(window_class, L"#32770") == 0)) {
    paint_surface(window, reinterpret_cast<HDC>(wp));
    return TRUE;
  }
  const bool progress = lstrcmpiW(window_class, PROGRESS_CLASSW) == 0;
  if ((role(window) == 2 || role(window) == 8 || role(window) == 10 || progress) && (message == WM_PAINT || message == WM_PRINTCLIENT)) {
    PAINTSTRUCT paint{};
    HDC dc = message == WM_PAINT ? BeginPaint(window, &paint) : reinterpret_cast<HDC>(wp);
    if (progress) paint_progress(window, dc);
    else if (role(window) == 2) paint_brand(window, dc);
    else if (role(window) == 10) {
      RECT bounds{};
      GetClientRect(window, &bounds);
      FillRect(dc, &bounds, background_brush);
      HGDIOBJ previous = SelectObject(dc, body_font);
      SetTextColor(dc, RGB(186, 186, 186));
      SetBkMode(dc, TRANSPARENT);
      const auto path = window_text(window);
      DrawTextW(dc, path.c_str(), -1, &bounds, DT_PATH_ELLIPSIS | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
      SelectObject(dc, previous);
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
    if (tag == 21) {
      details_open = !details_open;
      SetWindowTextW(reinterpret_cast<HWND>(lp), details_open ? L"Ocultar detalles" : L"Ver detalles");
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
  if (message == WM_GETDLGCODE &&
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
      DeleteObject(body_font);
      DeleteObject(caption_font);
      body_font = caption_font = nullptr;
      DeleteObject(log_brush);
      log_brush = nullptr;
      DeleteObject(background_brush);
      background_brush = nullptr;
      if (IsWindow(tooltips)) DestroyWindow(tooltips);
      tooltips = nullptr;
      current_footer = nullptr;
      hovered_button = nullptr;
      details_open = false;
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
  SetWindowSubclass(window, presentation, 1, 0);
  wchar_t type[32]{};
  GetClassNameW(window, type, 32);
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
  if (tag == 5) {
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
      SetWindowLongPtrW(window, GWL_STYLE, (style & ~BS_TYPEMASK) | BS_OWNERDRAW);
      InvalidateRect(window, nullptr, TRUE);
    }
  }
  return TRUE;
}

void apply_presentation(HWND parent) {
  if (!IsWindow(parent)) return;
  root = parent;
  if (!body_font) {
    const UINT dpi = GetDpiForWindow(root);
    body_font = CreateFontW(-MulDiv(11, dpi, 72), 0, 0, 0, FW_NORMAL, FALSE,
        FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY, DEFAULT_PITCH, L"Segoe UI");
    caption_font = CreateFontW(-MulDiv(9, dpi, 72), 0, 0, 0, FW_NORMAL, FALSE,
        FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY, DEFAULT_PITCH, L"Segoe UI");
  }
  if (!log_brush) log_brush = CreateSolidBrush(card);
  if (!background_brush) background_brush = CreateSolidBrush(background);
  SetWindowSubclass(root, presentation, 1, 0);
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
