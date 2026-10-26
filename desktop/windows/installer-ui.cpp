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
constexpr COLORREF background = RGB(16, 35, 30);
constexpr COLORREF card = RGB(27, 57, 46);
constexpr COLORREF gold = RGB(242, 207, 116);
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
using Controls = std::array<HWND, 21>;
void apply_presentation(HWND parent);
int phase() { return static_cast<int>(reinterpret_cast<INT_PTR>(GetPropW(root, L"PanitaPhase"))); }

int scale(int value) { return MulDiv(value, static_cast<int>(GetDpiForWindow(root)), 96); }
int role(HWND window) {
  return static_cast<int>(reinterpret_cast<INT_PTR>(GetPropW(window, L"PanitaRole")));
}
void draw_logo(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  HBRUSH brush = CreateSolidBrush(background);
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
void resize_shell(HWND page) {
  constexpr int height = 600;
  SetWindowPos(root, nullptr, 0, 0, scale(450), scale(height),
      SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
  place(page, 24, 24, 402, height - 116);
  place(GetDlgItem(root, IDOK), 24, height - 68, 402, 44);
}
void position_header(const Controls& controls) {
  place(controls[1], 0, 0, 32, 32);
  place(controls[2], 48, 0, 222, 32);
  place(controls[3], 330, 0, 32, 32);
  place(controls[4], 370, 0, 32, 32);
  place(controls[5], 0, 44, 402, 20);
}
void compact_page(const Controls& controls) {
  if (!controls[6] || !controls[7]) return;
  HWND page = GetParent(controls[6]);
  position_header(controls);
  int cursor = 80;
  auto row = [&](int id, int gap, int minimum, int width = 402) {
    HWND control = controls[static_cast<size_t>(id)];
    if (!visible(control)) return;
    const int height = (std::max)(minimum, text_height(control, width));
    place(control, 0, cursor, 402, height);
    cursor += height + gap;
  };
  row(6, 10, 32);
  row(7, 16, 18);
  if (controls[8] && controls[9]) {
    const int height = text_height(controls[9], 370);
    place(controls[8], 0, cursor, 402, height + 24);
    place(controls[9], 16, cursor + 12, 370, height);
    cursor += height + 40;
  }
  row(10, 12, 18);
  row(11, 16, 40);
  row(12, 0, 28, 374);
  row(13, 12, 36, 374);
  row(14, 0, 18);
  row(15, 12, 36, 374);
  row(16, 0, 32);
  row(18, 12, 44, 374);
  row(19, 0, 44, 374);
  row(20, 0, 36, 374);
  resize_shell(page);
}

// Native NSIS controls remain authoritative for whether a failed worker has
// finished. In particular, failure enables Cancel, not the disabled Next button.
void refresh_actions() {
  HWND next = GetDlgItem(root, IDOK);
  HWND cancel = GetDlgItem(root, IDCANCEL);
  HWND back = GetDlgItem(root, 3);
  if (visible(back)) ShowWindow(back, SW_HIDE);
  const int state = phase();
  if (state == 3 || state == 4) EnableWindow(cancel, TRUE);
  const bool failed_close = state == 2 && IsWindowEnabled(cancel);
  HWND action = failed_close ? cancel : next;
  if (visible(action == cancel ? next : cancel)) ShowWindow(action == cancel ? next : cancel, SW_HIDE);
  ShowWindow(action, SW_SHOWNA);
  place(action, 24, 532, 402, 44);
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
    controls[2] = header_control(page, L"STATIC", L"Panita Client", 2, SS_CENTERIMAGE);
    controls[3] = header_control(page, L"BUTTON", L"Minimizar", 3, BS_PUSHBUTTON | WS_TABSTOP);
    controls[4] = header_control(page, L"BUTTON", L"Cerrar", 4, BS_PUSHBUTTON | WS_TABSTOP);
    const auto version = window_text(static_cast<HWND>(GetPropW(root, L"PanitaVersionLabel")));
    controls[5] = header_control(page, L"STATIC", version.c_str(), 5);
    HFONT font = CreateFontW(-MulDiv(17, GetDpiForWindow(root), 72), 0, 0, 0,
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
  const bool warning = visible(controls[17]);
  place(controls[17], 0, 124, 402, 108);
  place(GetDlgItem(page, 1006), 0, 124, 402, 36);
  place(GetDlgItem(page, 1004), 0, 174, 402, 10);
  place(wrapped_log, 0, warning ? 248 : 200, 402, warning ? 220 : 268);
  // The stage title uses its own role so it cannot replace the shared brand.
  for (HWND child = GetWindow(page, GW_CHILD); child; child = GetWindow(child, GW_HWNDNEXT))
    if (GetPropW(child, L"PanitaStageTitle")) place(child, 0, 80, 402, 32);
  resize_shell(page);
  sync_log();
}

bool checkbox(HWND window) {
  wchar_t type[32]{};
  GetClassNameW(window, type, 32);
  if (lstrcmpiW(type, L"Button") != 0) return false;
  const LONG_PTR kind = GetWindowLongPtrW(window, GWL_STYLE) & BS_TYPEMASK;
  return kind == BS_CHECKBOX || kind == BS_AUTOCHECKBOX;
}

void draw_checkbox(HWND window, HDC dc) {
  RECT bounds{};
  GetClientRect(window, &bounds);
  const UINT dpi = GetDpiForWindow(root);
  HBRUSH base = CreateSolidBrush(background);
  FillRect(dc, &bounds, base);
  DeleteObject(base);
  const int size = MulDiv(18, dpi, 96);
  const bool checked = SendMessageW(window, BM_GETCHECK, 0, 0) == BST_CHECKED;
  RECT box{0, (bounds.bottom - size) / 2, size, (bounds.bottom + size) / 2};
  HBRUSH fill = CreateSolidBrush(checked ? gold : card);
  FillRect(dc, &box, fill);
  DeleteObject(fill);
  HBRUSH border = CreateSolidBrush(gold);
  FrameRect(dc, &box, border);
  DeleteObject(border);
  if (checked) {
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
  SetTextColor(dc, IsWindowEnabled(window) ? RGB(239, 236, 229) : RGB(135, 154, 143));
  RECT text_bounds = bounds;
  text_bounds.left = size + MulDiv(10, dpi, 96);
  RECT measured = text_bounds;
  DrawTextW(dc, text, -1, &measured, DT_CALCRECT | DT_WORDBREAK);
  text_bounds.top = (bounds.bottom - (measured.bottom - measured.top)) / 2;
  DrawTextW(dc, text, -1, &text_bounds, DT_WORDBREAK);
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
  HBRUSH base = CreateSolidBrush(background);
  FillRect(item.hDC, &item.rcItem, base);
  DeleteObject(base);
  const COLORREF fill = disabled ? card : primary ? (pressed ? RGB(211, 176, 85) : gold)
                                                    : (pressed ? RGB(42, 76, 62) : card);
  HBRUSH brush = CreateSolidBrush(fill);
  HPEN pen = CreatePen(PS_SOLID, 1, fill);
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
  SetTextColor(item.hDC, disabled ? RGB(135, 154, 143)
                               : primary ? background : RGB(239, 236, 229));
  RECT bounds = item.rcItem;
  const int tag = role(item.hwndItem);
  if (tag == 3 || tag == 4) {
    const int center_x = (bounds.left + bounds.right) / 2;
    const int center_y = (bounds.top + bounds.bottom) / 2;
    const int half = scale(5);
    HPEN icon_pen = CreatePen(PS_SOLID, (std::max)(1, scale(1)),
        disabled ? RGB(135, 154, 143) : RGB(239, 236, 229));
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
    DrawTextW(item.hDC, text, -1, &bounds, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
  }
  if ((item.itemState & ODS_FOCUS) && !(item.itemState & ODS_NOFOCUSRECT)) {
    InflateRect(&bounds, -4, -4);
    DrawFocusRect(item.hDC, &bounds);
  }
  SelectObject(item.hDC, old_font);
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
  if (message == WM_COMMAND && HIWORD(wp) == BN_CLICKED && lp) {
    const int tag = role(reinterpret_cast<HWND>(lp));
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
      (window == GetDlgItem(root, 3) || (window == GetDlgItem(root, IDCANCEL) && phase() != 2))) {
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
      SetTextColor(reinterpret_cast<HDC>(wp), tag == 5 ? gold : RGB(239, 236, 229));
      SetBkColor(reinterpret_cast<HDC>(wp), background);
      return reinterpret_cast<LRESULT>(background_brush);
    }
  }
  if (message == WM_CTLCOLORSTATIC && GetPropW(reinterpret_cast<HWND>(lp), L"PanitaWrappedLog")) {
    HDC dc = reinterpret_cast<HDC>(wp);
    SetTextColor(dc, RGB(239, 236, 229));
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
  // Keep the native auto-checkbox style/state and accessibility provider. Paint
  // it without the white themed square, including wrapped text and keyboard focus.
  if (checkbox(window)) {
    if (message == WM_PAINT) {
      PAINTSTRUCT paint{};
      HDC dc = BeginPaint(window, &paint);
      draw_checkbox(window, dc);
      EndPaint(window, &paint);
      return 0;
    }
    if (message == WM_PRINTCLIENT) {
      draw_checkbox(window, reinterpret_cast<HDC>(wp));
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
  if (font.lfHeight >= -MulDiv(14, GetDpiForWindow(root), 96)) {
    RECT bounds{};
    GetWindowRect(window, &bounds);
    const bool compact_button = lstrcmpiW(type, L"Button") == 0 && !checkbox(window) &&
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
