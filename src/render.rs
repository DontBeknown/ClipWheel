//! GPU-backed transparent composition surface. All COM objects stay on the UI thread.
use clipwheel::model::{Entry, Payload};
use std::{collections::HashMap, f32::consts::TAU, sync::Arc};
use windows::{
    core::*,
    Foundation::Numerics::Matrix3x2,
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::*,
            Direct3D11::*,
            DirectComposition::*,
            DirectWrite::*,
            Dxgi::{Common::*, *},
        },
    },
};

pub const WIDTH: f32 = 560.;
pub const HEIGHT: f32 = 332.;
pub const CENTER: (f32, f32) = (156., 156.);
const LABEL_WIDTH: f32 = 60.;
const LABEL_HEIGHT: f32 = 20.;
const CONTENT_RADIUS: f32 = 100.;
const TIP_LEFT: f32 = 308.;
const TIP_CONTENT_WIDTH: f32 = 212.;
const TIP_CONTENT_HEIGHT: f32 = 152.;
fn slice_size(entry: &Entry) -> (f32, f32) {
    if matches!(entry.payload.as_ref(), Payload::Image { .. }) {
        (32., 32.)
    } else {
        (LABEL_WIDTH, LABEL_HEIGHT)
    }
}
fn contains(area: D2D_RECT_F, x: f32, y: f32) -> bool {
    (area.left..=area.right).contains(&x) && (area.top..=area.bottom).contains(&y)
}
fn palette(hex: u32) -> D2D1_COLOR_F {
    color(
        ((hex >> 16) & 255) as f32 / 255.,
        ((hex >> 8) & 255) as f32 / 255.,
        (hex & 255) as f32 / 255.,
        1.,
    )
}

pub struct Frame<'a> {
    pub entries: &'a [Arc<Entry>],
    pub page: usize,
    pub total_pages: usize,
    pub hover: Option<usize>,
    pub hover_amount: f32,
    pub preview: bool,
    pub scroll: f32,
    pub section: usize,
    pub opening: f32,
}

// Slice layouts are measured once, using the same DirectWrite format as drawing.
struct SliceLayout {
    overflow: bool,
    layout: Option<IDWriteTextLayout>,
}
unsafe fn measure_slice(
    dw: &IDWriteFactory,
    format: &IDWriteTextFormat,
    entry: &Entry,
    width: f32,
    height: f32,
) -> Result<SliceLayout> {
    if let Payload::Image {
        width: w,
        height: h,
        ..
    } = entry.payload.as_ref()
    {
        return Ok(SliceLayout {
            overflow: *w as f32 > width || *h as f32 > height,
            layout: None,
        });
    }
    // A multi-section entry necessarily exceeds this tiny viewport. Measuring its
    // first section bounds UI-thread work while still testing all potentially fitting content.
    let text = entry.section(0);
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let mut layout = dw.CreateTextLayout(&utf16, format, width, 16_000_000.)?;
    layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
    layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
    let mut metrics = DWRITE_TEXT_METRICS::default();
    layout.GetMetrics(&mut metrics)?;
    let overflow = entry.section_count() > 1
        || metrics.height > height
        || metrics.widthIncludingTrailingWhitespace > width;
    if overflow {
        // Native vertical trimming does not mark explicit-newline truncation reliably.
        // Find a Unicode-safe prefix whose final ellipsis also fits the same viewport.
        let boundaries: Vec<usize> = text
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(text.len()))
            .collect();
        let mut low = 0;
        let mut high = boundaries.len() - 1;
        while low < high {
            let mid = (low + high).div_ceil(2);
            let candidate = format!("{}\u{2026}", text[..boundaries[mid]].trim_end());
            let candidate = dw.CreateTextLayout(
                &candidate.encode_utf16().collect::<Vec<_>>(),
                format,
                width,
                16_000_000.,
            )?;
            candidate.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            candidate.GetMetrics(&mut metrics)?;
            if metrics.height <= height && metrics.widthIncludingTrailingWhitespace <= width {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        let display = format!("{}\u{2026}", text[..boundaries[low]].trim_end());
        layout = dw.CreateTextLayout(
            &display.encode_utf16().collect::<Vec<_>>(),
            format,
            width,
            height,
        )?;
        layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
    }
    Ok(SliceLayout {
        overflow,
        layout: Some(layout),
    })
}

pub struct Renderer {
    factory: ID2D1Factory1,
    dc: ID2D1DeviceContext,
    swap: IDXGISwapChain1,
    _composition: IDCompositionDevice,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    dw: IDWriteFactory,
    format: IDWriteTextFormat,
    preview_rect: Option<D2D_RECT_F>,
    preview_viewport: f32,
    slices: HashMap<(u64, u32, u32), SliceLayout>,
    brush: ID2D1SolidColorBrush,
    images: HashMap<u64, ID2D1Bitmap1>,
    layout: Option<(u64, usize, IDWriteTextLayout, f32)>,
    scale: f32,
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
fn point(x: f32, y: f32) -> D2D_POINT_2F {
    D2D_POINT_2F { x, y }
}

impl Renderer {
    pub fn new(hwnd: HWND, scale: f32) -> Result<Self> {
        unsafe {
            let mut d3d = None;
            let make = |driver, out: &mut Option<ID3D11Device>| {
                D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(out),
                    None,
                    None,
                )
            };
            make(D3D_DRIVER_TYPE_HARDWARE, &mut d3d)
                .or_else(|_| make(D3D_DRIVER_TYPE_WARP, &mut d3d))?;
            let d3d = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let dx_factory: IDXGIFactory2 = adapter.GetParent()?;
            let swap = dx_factory.CreateSwapChainForComposition(
                &d3d,
                &DXGI_SWAP_CHAIN_DESC1 {
                    Width: (WIDTH * scale).ceil() as u32,
                    Height: (HEIGHT * scale).ceil() as u32,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                    BufferCount: 2,
                    Scaling: DXGI_SCALING_STRETCH,
                    SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                    AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                    ..Default::default()
                },
                None,
            )?;
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let device = factory.CreateDevice(&dxgi)?;
            let dc = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let surface: IDXGISurface = swap.GetBuffer(0)?;
            let bitmap = dc.CreateBitmapFromDxgiSurface(
                &surface,
                Some(&D2D1_BITMAP_PROPERTIES1 {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    dpiX: 96. * scale,
                    dpiY: 96. * scale,
                    bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                    ..Default::default()
                }),
            )?;
            dc.SetTarget(&bitmap);
            dc.SetDpi(96. * scale, 96. * scale);
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = composition.CreateTargetForHwnd(hwnd, true)?;
            let visual = composition.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            composition.Commit()?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let font = |size| {
                dw.CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )
            };
            let format = font(14.)?;
            let brush = dc.CreateSolidColorBrush(&color(1., 1., 1., 1.), None)?;
            Ok(Self {
                factory,
                dc,
                swap,
                _composition: composition,
                _target: target,
                _visual: visual,
                dw,
                format,
                preview_rect: None,
                preview_viewport: TIP_CONTENT_HEIGHT,
                slices: HashMap::new(),
                brush,
                images: HashMap::new(),
                layout: None,
                scale,
            })
        }
    }

    unsafe fn ensure_slice(&mut self, entry: &Entry, width: f32, height: f32) -> Result<()> {
        let key = (entry.id, width.to_bits(), height.to_bits());
        if !self.slices.contains_key(&key) {
            self.slices.insert(
                key,
                measure_slice(&self.dw, &self.format, entry, width, height)?,
            );
        }
        Ok(())
    }
    pub fn needs_preview(&mut self, entry: &Entry) -> Result<bool> {
        let (w, h) = slice_size(entry);
        unsafe {
            self.ensure_slice(entry, w, h)?;
        }
        Ok(self.slices[&(entry.id, w.to_bits(), h.to_bits())].overflow)
    }
    pub fn preview_contains(&self, x: f32, y: f32) -> bool {
        self.preview_rect.is_some_and(|r| contains(r, x, y))
    }
    pub fn preview_bridge(&self, x: f32, y: f32) -> bool {
        self.preview_rect
            .is_some_and(|r| x >= CENTER.0 && x <= r.right && y >= r.top && y <= r.bottom)
    }
    unsafe fn panel(&self, area: D2D_RECT_F) {
        let rounded = D2D1_ROUNDED_RECT {
            rect: area,
            radiusX: 10.,
            radiusY: 10.,
        };
        self.brush.SetColor(&palette(0x313244));
        self.dc.FillRoundedRectangle(&rounded, &self.brush);
        self.brush.SetColor(&palette(0x585b70));
        self.dc
            .DrawRoundedRectangle(&rounded, &self.brush, 1., None);
    }
    unsafe fn logo(&self) {
        let x = CENTER.0;
        let y = CENTER.1 - 5.;
        self.brush.SetColor(&palette(0xcdd6f4));
        self.dc.DrawRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(x - 7., y - 8., 14., 18.),
                radiusX: 2.,
                radiusY: 2.,
            },
            &self.brush,
            1.5,
            None,
        );
        self.brush.SetColor(&palette(0x313244));
        let clip = D2D1_ROUNDED_RECT {
            rect: rect(x - 3.5, y - 10., 7., 4.),
            radiusX: 1.5,
            radiusY: 1.5,
        };
        self.dc.FillRoundedRectangle(&clip, &self.brush);
        self.brush.SetColor(&palette(0xb4befe));
        self.dc.DrawRoundedRectangle(&clip, &self.brush, 1.5, None);
    }
    unsafe fn icon(&self, payload: &Payload, x: f32, y: f32, tint: D2D1_COLOR_F) {
        self.brush.SetColor(&tint);
        let line = |a, b| self.dc.DrawLine(a, b, &self.brush, 1.5, None);
        match payload {
            Payload::Text(_) => {
                self.dc.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(x - 8., y - 8., 16., 16.),
                        radiusX: 3.,
                        radiusY: 3.,
                    },
                    &self.brush,
                    1.3,
                    None,
                );
                line(point(x - 4., y - 3.), point(x + 4., y - 3.));
                line(point(x, y - 3.), point(x, y + 4.));
                line(point(x - 2., y + 4.), point(x + 2., y + 4.));
            }
            Payload::Files(_) => {
                self.dc.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(x - 9., y - 4., 18., 12.),
                        radiusX: 2.,
                        radiusY: 2.,
                    },
                    &self.brush,
                    1.5,
                    None,
                );
                line(point(x - 8., y - 4.), point(x - 8., y - 7.));
                line(point(x - 8., y - 7.), point(x - 1., y - 7.));
                line(point(x - 1., y - 7.), point(x + 2., y - 4.));
            }
            Payload::Image { .. } => {
                self.dc.DrawRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: rect(x - 9., y - 7., 18., 14.),
                        radiusX: 2.,
                        radiusY: 2.,
                    },
                    &self.brush,
                    1.5,
                    None,
                );
                line(point(x - 7., y + 5.), point(x - 1., y - 1.));
                line(point(x - 1., y - 1.), point(x + 3., y + 3.));
                line(point(x + 3., y + 3.), point(x + 7., y));
            }
        }
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }
    pub fn clear_images(&mut self) {
        self.images.clear();
        self.slices.clear();
        self.layout = None;
        self.preview_rect = None;
    }
    pub fn max_scroll(&self) -> f32 {
        self.layout
            .as_ref()
            .map_or(0., |(_, _, _, h)| (h - self.preview_viewport).max(0.))
    }

    unsafe fn text(
        &self,
        s: &str,
        area: D2D_RECT_F,
        _size: u8,
        tint: D2D1_COLOR_F,
        centered: bool,
    ) {
        let format = &self.format;
        let _ = format.SetTextAlignment(if centered {
            DWRITE_TEXT_ALIGNMENT_CENTER
        } else {
            DWRITE_TEXT_ALIGNMENT_LEADING
        });
        self.brush.SetColor(&tint);
        self.dc.DrawText(
            &s.encode_utf16().collect::<Vec<_>>(),
            format,
            &area,
            &self.brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL,
        );
    }

    unsafe fn sector(&self, i: usize, n: usize, grow: f32, _opacity: f32) -> Result<()> {
        let step = TAU / n as f32;
        let mid = -TAU / 4. + i as f32 * step;
        let gap = 0.;
        let a = mid - step / 2. + gap;
        let b = mid + step / 2. - gap;
        let inner = clipwheel::geometry::INNER_RADIUS;
        let outer = clipwheel::geometry::OUTER_RADIUS;
        let path = self.factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(
            point(CENTER.0 + inner * a.cos(), CENTER.1 + inner * a.sin()),
            D2D1_FIGURE_BEGIN_FILLED,
        );
        let mut pts = Vec::with_capacity(100);
        for k in 0..=48 {
            let t = a + (b - a) * k as f32 / 48.;
            pts.push(point(
                CENTER.0 + outer * t.cos(),
                CENTER.1 + outer * t.sin(),
            ));
        }
        for k in (0..=48).rev() {
            let t = a + (b - a) * k as f32 / 48.;
            pts.push(point(
                CENTER.0 + inner * t.cos(),
                CENTER.1 + inner * t.sin(),
            ));
        }
        sink.AddLines(&pts);
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        let base = palette(0x1e1e2e);
        let selected = palette(0x313244);
        self.brush.SetColor(&color(
            base.r + (selected.r - base.r) * grow,
            base.g + (selected.g - base.g) * grow,
            base.b + (selected.b - base.b) * grow,
            1.,
        ));
        self.dc.FillGeometry(&path, &self.brush, None);
        self.brush
            .SetColor(&palette(if grow > 0. { 0xb4befe } else { 0x45475a }));
        self.dc
            .DrawGeometry(&path, &self.brush, if grow > 0. { 1.3 } else { 0.7 }, None);
        Ok(())
    }

    unsafe fn image(&mut self, entry: &Entry, area: D2D_RECT_F, opacity: f32) -> Result<()> {
        if let Some(clipwheel::model::Thumbnail {
            width,
            height,
            bgra,
        }) = &entry.thumbnail
        {
            if !self.images.contains_key(&entry.id) {
                let bitmap = self.dc.CreateBitmap(
                    D2D_SIZE_U {
                        width: *width,
                        height: *height,
                    },
                    Some(bgra.as_ptr().cast()),
                    width * 4,
                    &D2D1_BITMAP_PROPERTIES1 {
                        pixelFormat: D2D1_PIXEL_FORMAT {
                            format: DXGI_FORMAT_B8G8R8A8_UNORM,
                            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                        },
                        dpiX: 96.,
                        dpiY: 96.,
                        ..Default::default()
                    },
                )?;
                self.images.insert(entry.id, bitmap);
            }
            let ratio = ((area.right - area.left) / *width as f32)
                .min((area.bottom - area.top) / *height as f32);
            let w = *width as f32 * ratio;
            let h = *height as f32 * ratio;
            let dest = rect(
                area.left + (area.right - area.left - w) / 2.,
                area.top + (area.bottom - area.top - h) / 2.,
                w,
                h,
            );
            self.dc.DrawBitmap(
                self.images.get(&entry.id).unwrap(),
                Some(&dest),
                opacity,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
        }
        Ok(())
    }

    pub fn draw(&mut self, frame: Frame<'_>) -> Result<()> {
        self.draw_frame(frame, true)
    }
    fn draw_frame(&mut self, frame: Frame<'_>, present: bool) -> Result<()> {
        unsafe {
            self.preview_rect = None;
            self.dc.BeginDraw();
            self.dc.Clear(Some(&color(0., 0., 0., 0.)));
            let p = frame.opening.clamp(0., 1.);
            let scale = 0.96 + 0.04 * (1. - (1. - p).powi(3));
            self.dc.SetTransform(&Matrix3x2 {
                M11: scale,
                M12: 0.,
                M21: 0.,
                M22: scale,
                M31: CENTER.0 * (1. - scale),
                M32: CENTER.1 * (1. - scale),
            });
            let text = palette(0xcdd6f4);
            let muted = palette(0xa6adc8);
            let accent = palette(0xb4befe);
            let n = frame.entries.len();
            if n > 0 {
                self.brush.SetColor(&palette(0x1e1e2e));
                self.dc.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: point(CENTER.0, CENTER.1),
                        radiusX: 140.,
                        radiusY: 140.,
                    },
                    &self.brush,
                );
            }
            // Draw surfaces first so selected borders remain continuous over neighboring edges.
            for i in 0..n {
                self.sector(i, n, 0., 1.)?;
            }
            if let Some(i) = frame.hover.filter(|i| *i < n) {
                self.sector(i, n, frame.hover_amount, 1.)?;
            }
            for (i, entry) in frame.entries.iter().enumerate() {
                let mid = -TAU / 4. + i as f32 * TAU / n as f32;
                let x = CENTER.0 + CONTENT_RADIUS * mid.cos();
                let y = CENTER.1 + CONTENT_RADIUS * mid.sin();
                let (w, h) = slice_size(entry);
                self.ensure_slice(entry, w, h)?;
                if matches!(entry.payload.as_ref(), Payload::Image { .. }) {
                    self.image(entry, rect(x - w / 2., y - h / 2., w, h), 1.)?;
                } else {
                    let tint = if frame.hover == Some(i) { accent } else { text };
                    self.icon(entry.payload.as_ref(), x, y - 10., tint);
                    let area = rect(x - w / 2., y + 2., w, h);
                    if let Some(layout) = &self.slices[&(entry.id, w.to_bits(), h.to_bits())].layout
                    {
                        self.dc
                            .PushAxisAlignedClip(&area, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                        self.brush.SetColor(&tint);
                        self.dc.DrawTextLayout(
                            point(area.left, area.top),
                            layout,
                            &self.brush,
                            D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        );
                        self.dc.PopAxisAlignedClip();
                    }
                }
            }
            let hub = D2D1_ELLIPSE {
                point: point(CENTER.0, CENTER.1),
                radiusX: 26.,
                radiusY: 26.,
            };
            self.brush.SetColor(&palette(0x313244));
            self.dc.FillEllipse(&hub, &self.brush);
            self.brush.SetColor(&palette(0x585b70));
            self.dc.DrawEllipse(&hub, &self.brush, 0.8, None);
            self.logo();
            for page in 0..frame.total_pages {
                let x = CENTER.0 + (page as f32 - (frame.total_pages - 1) as f32 / 2.) * 7.;
                self.brush.SetColor(&if page == frame.page {
                    accent
                } else {
                    palette(0x6c7086)
                });
                self.dc.FillEllipse(
                    &D2D1_ELLIPSE {
                        point: point(x, CENTER.1 + 15.),
                        radiusX: 2.,
                        radiusY: 2.,
                    },
                    &self.brush,
                );
            }
            // A crisp dark outline keeps the small hint readable on light desktops without a pill.
            for (dx, dy) in [(-1., 0.), (1., 0.), (0., -1.), (0., 1.)] {
                self.text(
                    "Scroll · Esc",
                    rect(66. + dx, 306. + dy, 180., 22.),
                    0,
                    palette(0x181825),
                    true,
                );
            }
            self.text("Scroll · Esc", rect(66., 306., 180., 22.), 0, muted, true);
            if n == 0 {
                self.panel(rect(36., 208., 240., 64.));
                self.text(
                    "Copy text, images or files\nto fill your wheel.",
                    rect(48., 220., 216., 44.),
                    0,
                    text,
                    true,
                );
            }
            if frame.preview {
                if let Some((i, entry)) = frame
                    .hover
                    .and_then(|i| frame.entries.get(i).map(|e| (i, e)))
                {
                    if self.needs_preview(entry)? {
                        let mid = -TAU / 4. + i as f32 * TAU / n as f32;
                        let anchor_y = CENTER.1 + CONTENT_RADIUS * mid.sin();
                        match entry.payload.as_ref() {
                            Payload::Image { width, height, .. } => {
                                self.layout = None;
                                let ratio = (TIP_CONTENT_WIDTH / *width as f32)
                                    .min(TIP_CONTENT_HEIGHT / *height as f32)
                                    .min(1.);
                                let w = (*width as f32 * ratio).max(48.);
                                let h = (*height as f32 * ratio).max(32.);
                                let top = (anchor_y - (h + 28.) / 2.).clamp(12., HEIGHT - h - 40.);
                                let card = rect(TIP_LEFT, top, w + 28., h + 28.);
                                self.preview_rect = Some(card);
                                self.panel(card);
                                self.image(entry, rect(TIP_LEFT + 14., top + 14., w, h), 1.)?;
                            }
                            _ => {
                                if self.layout.as_ref().is_none_or(|(id, section, _, _)| {
                                    *id != entry.id || *section != frame.section
                                }) {
                                    let utf16: Vec<u16> =
                                        entry.section(frame.section).encode_utf16().collect();
                                    let layout = self.dw.CreateTextLayout(
                                        &utf16,
                                        &self.format,
                                        TIP_CONTENT_WIDTH,
                                        16_000_000.,
                                    )?;
                                    layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING)?;
                                    layout.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
                                    let mut metrics = DWRITE_TEXT_METRICS::default();
                                    layout.GetMetrics(&mut metrics)?;
                                    self.layout =
                                        Some((entry.id, frame.section, layout, metrics.height));
                                }
                                let (_, _, layout, height) = self.layout.as_ref().unwrap();
                                let mut metrics = DWRITE_TEXT_METRICS::default();
                                layout.GetMetrics(&mut metrics)?;
                                // Long entries keep fixed bounds across sections, so a short final
                                // section cannot move the scrolling surface away from the cursor.
                                let (w, h) = if entry.section_count() > 1 {
                                    (TIP_CONTENT_WIDTH, TIP_CONTENT_HEIGHT)
                                } else {
                                    (
                                        metrics
                                            .widthIncludingTrailingWhitespace
                                            .ceil()
                                            .clamp(96., TIP_CONTENT_WIDTH),
                                        height.clamp(20., TIP_CONTENT_HEIGHT),
                                    )
                                };
                                self.preview_viewport = h;
                                let top = (anchor_y - (h + 28.) / 2.).clamp(12., HEIGHT - h - 40.);
                                let card = rect(TIP_LEFT, top, w + 28., h + 28.);
                                self.preview_rect = Some(card);
                                self.panel(card);
                                let area = rect(TIP_LEFT + 14., top + 14., w, h);
                                self.dc
                                    .PushAxisAlignedClip(&area, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                                self.brush.SetColor(&text);
                                self.dc.DrawTextLayout(
                                    point(
                                        area.left,
                                        area.top - frame.scroll.min((height - h).max(0.)),
                                    ),
                                    layout,
                                    &self.brush,
                                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                                );
                                self.dc.PopAxisAlignedClip();
                                if *height > h {
                                    let thumb = (h * h / height).max(12.);
                                    let offset =
                                        frame.scroll.min(height - h) / (height - h) * (h - thumb);
                                    self.brush.SetColor(&palette(0x6c7086));
                                    self.dc.FillRoundedRectangle(
                                        &D2D1_ROUNDED_RECT {
                                            rect: rect(
                                                card.right - 6.,
                                                top + 14. + offset,
                                                2.,
                                                thumb,
                                            ),
                                            radiusX: 1.,
                                            radiusY: 1.,
                                        },
                                        &self.brush,
                                    );
                                }
                            }
                        }
                    }
                }
            }
            self.dc.EndDraw(None, None)?;
            if present {
                self.swap.Present(1, DXGI_PRESENT(0)).ok()?;
            }
            Ok(())
        }
    }
}
#[cfg(test)]
mod compact_tests {
    use super::*;
    fn entry(payload: Payload) -> Arc<Entry> {
        let mut history = clipwheel::model::History::new(32, 128 * 1024 * 1024, 32 * 1024 * 1024);
        history.insert(payload).unwrap();
        history.entries[0].clone()
    }
    #[test]
    fn overflow_uses_real_directwrite_metrics() {
        unsafe {
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let format = dw
                .CreateTextFormat(
                    w!("Segoe UI"),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    14.,
                    w!("en-us"),
                )
                .unwrap();
            for (text, expected) in [
                ("Hi", false),
                ("ไทย", false),
                ("one\ntwo\nthree\nfour", true),
                (
                    "A very long sentence that must wrap beyond the available slice content area",
                    true,
                ),
                ("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789", true),
            ] {
                let e = entry(Payload::Text(text.into()));
                assert_eq!(
                    measure_slice(&dw, &format, &e, 76., 50.).unwrap().overflow,
                    expected,
                    "{text}"
                );
            }
            let e = entry(Payload::Files(vec![
                "C:\\a.txt".into(),
                "C:\\a very long folder\\another file.txt".into(),
            ]));
            assert!(measure_slice(&dw, &format, &e, 76., 50.).unwrap().overflow);
            let e = entry(Payload::Text("x".repeat(5000)));
            assert!(measure_slice(&dw, &format, &e, 76., 50.).unwrap().overflow);
            let e = entry(Payload::Text("Fits after resizing".into()));
            assert!(measure_slice(&dw, &format, &e, 76., 20.).unwrap().overflow);
            assert!(!measure_slice(&dw, &format, &e, 300., 50.).unwrap().overflow);
            for (width, height, expected) in [
                (20, 20, false),
                (76, 50, false),
                (77, 50, true),
                (20, 51, true),
            ] {
                let e = entry(Payload::Image {
                    width,
                    height,
                    bgra: vec![255; (width * height * 4) as usize],
                });
                assert_eq!(
                    measure_slice(&dw, &format, &e, 76., 50.).unwrap().overflow,
                    expected
                );
            }
        }
    }
    #[test]
    fn content_rectangles_stay_inside_all_wedges() {
        for n in 1..=8 {
            for i in 0..n {
                let mid = -TAU / 4. + i as f32 * TAU / n as f32;
                for dx in [-30., 30.] {
                    for dy in [-18., 22.] {
                        let x = CONTENT_RADIUS * mid.cos() + dx;
                        let y = CONTENT_RADIUS * mid.sin() + dy;
                        assert_eq!(clipwheel::geometry::hit_segment(x, y, n), Some(i));
                        assert!(x.hypot(y) < 140.);
                    }
                }
            }
        }
    }

    // Explicit opt-in: creates a hidden native render target and exports real GPU pixels.
    #[test]
    #[ignore = "Exports renderer visual matrix; requires a Windows graphics device"]
    fn export_compact_visual_matrix() {
        use windows::Win32::System::Com::*;
        use windows::Win32::UI::WindowsAndMessaging::*;
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Glass render QA"),
                WS_POPUP,
                0,
                0,
                1640,
                960,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let folder = std::path::Path::new("docs/verification/compact");
            std::fs::create_dir_all(folder).unwrap();
            for scale in [1., 1.5, 2.] {
                let mut renderer = Renderer::new(hwnd, scale).unwrap();
                let mut history =
                    clipwheel::model::History::new(32, 128 * 1024 * 1024, 32 * 1024 * 1024);
                for payload in [
                    Payload::Text("Hi".into()),
                    Payload::Text("A long clipboard item that needs a full preview.\n".repeat(24)),
                    Payload::Text("ไทย 🦀 Unicode".into()),
                    Payload::Files(vec![
                        "C:\\a.txt".into(),
                        "C:\\long folder\\second file.txt".into(),
                    ]),
                    Payload::Image {
                        width: 120,
                        height: 80,
                        bgra: [190, 120, 55, 255].repeat(120 * 80),
                    },
                    Payload::Text("Meeting notes\nReview the new layout\nTomorrow at 14:30".into()),
                    Payload::Text("https://example.com/a-long-unbroken-link".into()),
                    Payload::Text("Small".into()),
                ] {
                    history.insert(payload).unwrap();
                }
                for (name, count, hover) in [
                    ("empty", 0, None),
                    ("single", 1, Some(0)),
                    ("eight-fit", 8, Some(0)),
                    ("eight-overflow", 8, Some(6)),
                    ("files", 8, Some(4)),
                    ("tooltip-short", 8, Some(2)),
                    ("image", 8, Some(3)),
                ] {
                    let entries = &history.entries[..count];
                    let preview =
                        hover.is_some_and(|i| renderer.needs_preview(&entries[i]).unwrap());
                    renderer
                        .draw_frame(
                            Frame {
                                entries,
                                page: 0,
                                total_pages: 1,
                                hover,
                                hover_amount: 1.,
                                preview,
                                scroll: 0.,
                                section: 0,
                                opening: 1.,
                            },
                            false,
                        )
                        .unwrap();
                    assert_eq!(
                        preview,
                        matches!(name, "eight-overflow" | "files" | "image" | "tooltip-short")
                    );
                    assert_eq!(renderer.format.GetFontSize(), 14.);
                    if let Some(card) = renderer.preview_rect {
                        assert!(preview);
                        assert!(card.right - card.left <= 240. && card.bottom - card.top <= 180.);
                        assert!(
                            card.left >= 0.
                                && card.right <= WIDTH
                                && card.top >= 0.
                                && card.bottom <= HEIGHT
                        );
                        assert!(renderer.preview_contains(
                            (card.left + card.right) / 2.,
                            (card.top + card.bottom) / 2.
                        ));
                        assert!(!renderer.preview_contains(card.right + 1., card.top));
                    } else {
                        assert!(!preview);
                    }
                    let source: ID2D1Bitmap1 = renderer.dc.GetTarget().unwrap().cast().unwrap();
                    let size = source.GetPixelSize();
                    let bitmap = renderer
                        .dc
                        .CreateBitmap(
                            size,
                            None,
                            0,
                            &D2D1_BITMAP_PROPERTIES1 {
                                pixelFormat: source.GetPixelFormat(),
                                dpiX: 96. * scale,
                                dpiY: 96. * scale,
                                bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ
                                    | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                                ..Default::default()
                            },
                        )
                        .unwrap();
                    bitmap.CopyFromBitmap(None, &source, None).unwrap();
                    let mapped = bitmap.Map(D2D1_MAP_OPTIONS_READ).unwrap();
                    let mut pixels = Vec::new();
                    for y in 0..size.height {
                        pixels.extend_from_slice(std::slice::from_raw_parts(
                            mapped.bits.add((y * mapped.pitch) as usize),
                            (size.width * 4) as usize,
                        ));
                    }
                    bitmap.Unmap().unwrap();
                    assert!(pixels.iter().any(|&v| v != 0));
                    std::fs::write(
                        folder.join(format!(
                            "{name}-{}-{}x{}.bgra",
                            (scale * 100.) as u32,
                            size.width,
                            size.height
                        )),
                        pixels,
                    )
                    .unwrap();
                }
            }
            {
                let mut renderer = Renderer::new(hwnd, 1.).unwrap();
                let entry = entry(Payload::Text("x".repeat(4100)));
                let entries = [entry];
                let mut first = None;
                for section in [0, 1] {
                    renderer
                        .draw_frame(
                            Frame {
                                entries: &entries,
                                page: 0,
                                total_pages: 1,
                                hover: Some(0),
                                hover_amount: 1.,
                                preview: true,
                                scroll: 0.,
                                section,
                                opening: 1.,
                            },
                            false,
                        )
                        .unwrap();
                    let card = renderer.preview_rect.unwrap();
                    let bounds = (card.left, card.top, card.right, card.bottom);
                    if let Some(first) = first {
                        assert_eq!(
                            bounds, first,
                            "Changing sections must retain pointer ownership"
                        );
                    } else {
                        first = Some(bounds);
                    }
                }
            }
            DestroyWindow(hwnd).unwrap();
            CoUninitialize();
        }
    }
}
