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
HWND detail_source;
HWND wrapped_log;
bool log_sync_queued;
constexpr UINT sync_log_message = WM_APP + 42;

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
  auto& controls = *reinterpret_cast<std::array<HWND, 20>*>(data);
  const int id = role(window);
  if (id > 0 && id < static_cast<int>(controls.size())) controls[static_cast<size_t>(id)] = window;
  return TRUE;
}
void resize_shell(HWND page, int content_height) {
  const int height = (std::max)(360, content_height + 116);
  SetWindowPos(root, nullptr, 0, 0, scale(450), scale(height),
      SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
  place(page, 24, 24, 402, height - 116);
  place(GetDlgItem(root, IDOK), 24, height - 68, 402, 44);
}
void compact_page(const std::array<HWND, 20>& controls) {
  if (!controls[6] || !controls[7]) return;
  HWND page = GetParent(controls[6]);
  place(controls[1], 0, 0, 32, 32);
  place(controls[2], 48, 0, 222, 32);
  place(controls[3], 280, 2, 64, 28);
  place(controls[4], 350, 2, 52, 28);
  place(controls[5], 0, 44, 402, 20);
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
  resize_shell(page, cursor);
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
void prepare_log(const std::array<HWND, 20>& controls) {
  if (!controls[17] || controls[6] || !IsWindow(detail_source)) return;
  HWND page = GetParent(detail_source);
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
  place(wrapped_log, 0, warning ? 214 : 122, 402, warning ? 150 : 240);
  resize_shell(page, 364);
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

bool header_point(LPARAM position) {
  POINT point{GET_X_LPARAM(position), GET_Y_LPARAM(position)};
  ScreenToClient(root, &point);
  const UINT dpi = GetDpiForWindow(root);
  return point.y >= 0 && point.y < MulDiv(70, dpi, 96);
}

void draw_button(const DRAWITEMSTRUCT& item) {
  const bool primary = item.hwndItem == GetDlgItem(root, IDOK);
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
  DrawTextW(item.hDC, text, -1, &bounds, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
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
  if (message == WM_NCHITTEST && header_point(lp)) {
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
    if (window == detail_source) { detail_source = nullptr; wrapped_log = nullptr; log_sync_queued = false; }
    if (window == root) {
      DeleteObject(body_font);
      DeleteObject(caption_font);
      body_font = caption_font = nullptr;
      DeleteObject(log_brush);
      log_brush = nullptr;
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
}

extern "C" __declspec(dllexport) void __cdecl Apply(HWND parent, int, wchar_t*, void*, void*) {
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
  SetWindowSubclass(root, presentation, 1, 0);
  EnumChildWindows(root, style_child, 0);
  std::array<HWND, 20> controls{};
  EnumChildWindows(root, collect_roles, reinterpret_cast<LPARAM>(&controls));
  compact_page(controls);
  prepare_log(controls);
  RedrawWindow(root, nullptr, nullptr, RDW_INVALIDATE | RDW_ALLCHILDREN);
}
