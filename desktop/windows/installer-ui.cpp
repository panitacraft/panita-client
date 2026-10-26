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
    const bool compact_button = lstrcmpiW(type, L"Button") == 0 &&
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
