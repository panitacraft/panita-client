// Shared native presentation components for installer and uninstaller pages.
#pragma once

namespace installer_ui {
enum class Icon { close, minimize, arrow, trash, up, down, info, error, check };

inline Gdiplus::Color color(COLORREF value) {
  return Gdiplus::Color(255, GetRValue(value), GetGValue(value), GetBValue(value));
}

inline void surface(HDC dc, const RECT& bounds, COLORREF fill, COLORREF border, int radius) {
  Gdiplus::Graphics graphics(dc);
  graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
  const auto x = static_cast<Gdiplus::REAL>(bounds.left) + 0.5f;
  const auto y = static_cast<Gdiplus::REAL>(bounds.top) + 0.5f;
  const auto width = static_cast<Gdiplus::REAL>(bounds.right - bounds.left - 1);
  const auto height = static_cast<Gdiplus::REAL>(bounds.bottom - bounds.top - 1);
  const auto diameter = static_cast<Gdiplus::REAL>((std::min)(radius * 2, static_cast<int>((std::min)(width, height))));
  if (diameter <= 0) return;
  Gdiplus::GraphicsPath path;
  path.AddArc(x, y, diameter, diameter, 180, 90);
  path.AddArc(x + width - diameter, y, diameter, diameter, 270, 90);
  path.AddArc(x + width - diameter, y + height - diameter, diameter, diameter, 0, 90);
  path.AddArc(x, y + height - diameter, diameter, diameter, 90, 90);
  path.CloseFigure();
  Gdiplus::SolidBrush brush(color(fill));
  Gdiplus::Pen pen(color(border), 1.0f);
  graphics.FillPath(&brush, &path);
  graphics.DrawPath(&pen, &path);
}

// A keyboard focus ring: an outline only, drawn at the very edge so it stays visible on
// the green primary button as well as on dark ones. It never changes the control's fill.
inline void ring(HDC dc, const RECT& bounds, COLORREF stroke, int radius, float width) {
  Gdiplus::Graphics graphics(dc);
  graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
  const float inset = width / 2.0f;
  const auto x = static_cast<Gdiplus::REAL>(bounds.left) + inset;
  const auto y = static_cast<Gdiplus::REAL>(bounds.top) + inset;
  const auto w = static_cast<Gdiplus::REAL>(bounds.right - bounds.left) - width;
  const auto h = static_cast<Gdiplus::REAL>(bounds.bottom - bounds.top) - width;
  const auto diameter = static_cast<Gdiplus::REAL>((std::min)(radius * 2, static_cast<int>((std::min)(w, h))));
  if (diameter <= 0) return;
  Gdiplus::GraphicsPath path;
  path.AddArc(x, y, diameter, diameter, 180, 90);
  path.AddArc(x + w - diameter, y, diameter, diameter, 270, 90);
  path.AddArc(x + w - diameter, y + h - diameter, diameter, diameter, 0, 90);
  path.AddArc(x, y + h - diameter, diameter, diameter, 90, 90);
  path.CloseFigure();
  Gdiplus::Pen pen(color(stroke), width);
  graphics.DrawPath(&pen, &path);
}

// Draw every glyph from the same 16-unit vector grid at the actual window DPI.
inline void icon(HDC dc, Icon type, int x, int y, int size, COLORREF foreground) {
  Gdiplus::Graphics graphics(dc);
  graphics.SetSmoothingMode(Gdiplus::SmoothingModeAntiAlias);
  graphics.TranslateTransform(static_cast<float>(x), static_cast<float>(y));
  const float factor = static_cast<float>(size) / 16.0f;
  graphics.ScaleTransform(factor, factor);
  Gdiplus::Pen pen(color(foreground), 1.5f);
  pen.SetStartCap(Gdiplus::LineCapRound);
  pen.SetEndCap(Gdiplus::LineCapRound);
  pen.SetLineJoin(Gdiplus::LineJoinRound);
  auto line = [&](int x1, int y1, int x2, int y2) { graphics.DrawLine(&pen, x1, y1, x2, y2); };
  switch (type) {
    case Icon::close: line(3,3,13,13); line(3,13,13,3); break;
    case Icon::minimize: line(3,8,13,8); break;
    case Icon::arrow: line(2,8,14,8); line(9,3,14,8); line(14,8,9,13); break;
    case Icon::trash:
      line(2,4,14,4); line(6,1,10,1); line(4,4,4,14); line(4,14,12,14); line(12,14,12,4); break;
    case Icon::up: line(3,10,8,5); line(8,5,13,10); break;
    case Icon::down: line(3,5,8,10); line(8,10,13,5); break;
    case Icon::check: line(3,8,7,12); line(7,12,13,4); break;
    case Icon::info:
    case Icon::error:
      graphics.DrawEllipse(&pen, 1,1,14,14);
      line(8, type == Icon::error ? 4 : 7, 8, type == Icon::error ? 8 : 12);
      {
        Gdiplus::SolidBrush dot(color(foreground));
        graphics.FillEllipse(&dot, 7.25f, type == Icon::error ? 10.25f : 3.25f, 1.5f, 1.5f);
      }
      break;
  }
}
}
