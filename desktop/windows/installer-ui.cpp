// Presentation-only NSIS plugin. No installation, filesystem or account logic.
// The parameter-free export follows NSIS's Unicode plugin ABI. /NOUNLOAD keeps
// the subclass procedures resident until the installer process exits.
#define UNICODE
#define _UNICODE
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <windowsx.h>
#include <commctrl.h>

namespace {
constexpr COLORREF background = RGB(16, 35, 30);
constexpr COLORREF card = RGB(27, 57, 46);
constexpr COLORREF gold = RGB(242, 207, 116);
HWND root;
HFONT body_font;
HFONT caption_font;

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
  return point.y >= 0 && point.y < MulDiv(90, dpi, 96);
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
    if (window == root) {
      DeleteObject(body_font);
      DeleteObject(caption_font);
      body_font = caption_font = nullptr;
    }
  }
  return DefSubclassProc(window, message, wp, lp);
}

BOOL CALLBACK style_child(HWND window, LPARAM) {
  SetWindowSubclass(window, presentation, 1, 0);
  wchar_t type[32]{};
  GetClassNameW(window, type, 32);
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
  SetWindowSubclass(root, presentation, 1, 0);
  EnumChildWindows(root, style_child, 0);
  RedrawWindow(root, nullptr, nullptr, RDW_INVALIDATE | RDW_ALLCHILDREN);
}
