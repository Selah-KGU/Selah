//! Native capsule panel construction.

use super::border::{rounded_rect_path, set_gradient_colors, set_gradient_locations};
use super::*;

pub(super) fn build_panel(ui: &mut CapsuleViews) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let theme = Theme::current();

    let visible = NSScreen::mainScreen(mtm)
        .as_ref()
        .map(|s| s.visibleFrame())
        .unwrap_or_else(|| NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1440.0, 900.0)));

    let center_x = visible.origin.x + visible.size.width / 2.0;
    let top_y = visible.origin.y + visible.size.height - TOP_MARGIN;
    let frame = NSRect::new(
        NSPoint::new(center_x - LISTEN_W / 2.0, top_y - LISTEN_H),
        NSSize::new(LISTEN_W, LISTEN_H),
    );

    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setFloatingPanel(true);
    panel.setBecomesKeyOnlyIfNeeded(true);
    panel.setWorksWhenModal(true);
    panel.setOpaque(false);
    panel.setHasShadow(false);
    panel.setHidesOnDeactivate(false);
    panel.setLevel(NSFloatingWindowLevel);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient,
    );
    unsafe { panel.setReleasedWhenClosed(false) };

    let root = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)),
    );
    root.setWantsLayer(true);
    if let Some(layer) = root.layer() {
        layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
    }

    let capsule = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)),
    );
    capsule.setWantsLayer(true);
    if let Some(layer) = capsule.layer() {
        layer.setCornerRadius(CORNER_RADIUS);
        layer.setBackgroundColor(Some(&NSColor::clearColor().CGColor()));
        // Subtle drop shadow for depth.
        let (sr, sg, sb) = if theme.is_dark {
            (6, 4, 12)
        } else {
            (60, 40, 120)
        };
        layer.setShadowColor(Some(&srgb(sr, sg, sb, 0.62).CGColor()));
        layer.setShadowOffset(NSSize::new(0.0, -6.0));
        layer.setShadowRadius(22.0);
        layer.setShadowOpacity(if theme.is_dark { 0.30 } else { 0.14 });
    }

    let vfx: Retained<NSVisualEffectView> = unsafe {
        msg_send![
            NSVisualEffectView::alloc(mtm),
            initWithFrame: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H))
        ]
    };
    vfx.setMaterial(NSVisualEffectMaterial::HUDWindow);
    vfx.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    vfx.setState(NSVisualEffectState::Active);
    vfx.setWantsLayer(true);
    if let Some(layer) = vfx.layer() {
        layer.setCornerRadius(CORNER_RADIUS);
        layer.setMasksToBounds(true);
        let (r, g, b, a) = theme.border_idle();
        layer.setBorderColor(Some(&srgb(r, g, b, a).CGColor()));
        layer.setBorderWidth(BORDER_IDLE_W);
    }

    let bg = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(LISTEN_W, LISTEN_H)),
    );
    bg.setWantsLayer(true);
    if let Some(layer) = bg.layer() {
        let (r, g, b, a) = theme.background();
        layer.setBackgroundColor(Some(&srgb(r, g, b, a).CGColor()));
        layer.setCornerRadius(CORNER_RADIUS);
    }
    vfx.addSubview(&bg);

    // Pulsing left-side listen indicator (small orb that breathes during listening).
    let listen_indicator = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(8.0, 8.0)),
    );
    listen_indicator.setWantsLayer(true);
    if let Some(layer) = listen_indicator.layer() {
        layer.setBackgroundColor(Some(&theme.listen_indicator().CGColor()));
        layer.setCornerRadius(4.0);
        layer.setShadowColor(Some(&theme.listen_indicator().CGColor()));
        layer.setShadowRadius(9.0);
        layer.setShadowOpacity(0.6);
        layer.setShadowOffset(NSSize::new(0.0, 0.0));
        layer.setOpacity(0.0);
    }
    vfx.addSubview(&listen_indicator);

    // Main text label
    let label = NSTextField::labelWithString(&NSString::from_str("話してください"), mtm);
    label.setTranslatesAutoresizingMaskIntoConstraints(true);
    label.setFrame(NSRect::new(
        NSPoint::new(PAD_X, PAD_Y),
        NSSize::new(LISTEN_W - PAD_X * 2.0, LISTEN_H - PAD_Y * 2.0),
    ));
    label.setWantsLayer(true);
    label.setTextColor(Some(&theme.muted_label()));
    label.setFont(Some(&NSFont::systemFontOfSize(LISTEN_FONT)));
    label.setAlignment(NSTextAlignment::Center);
    label.setMaximumNumberOfLines(2);
    label.setPreferredMaxLayoutWidth(LISTEN_W - PAD_X * 2.0);
    if let Some(cell) = label.cell() {
        cell.setUsesSingleLineMode(false);
        cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    }
    vfx.addSubview(&label);

    // Processing dots (stay in view tree, hidden when not processing)
    let mut processing_dots = Vec::new();
    for _ in 0..3 {
        let dot = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(
                NSPoint::new(0.0, 0.0),
                NSSize::new(PROCESS_DOT_SIZE, PROCESS_DOT_SIZE),
            ),
        );
        dot.setWantsLayer(true);
        if let Some(layer) = dot.layer() {
            layer.setBackgroundColor(Some(&theme.accent().CGColor()));
            layer.setCornerRadius(PROCESS_DOT_SIZE / 2.0);
            layer.setOpacity(0.0);
            layer.setShadowColor(Some(&theme.accent().CGColor()));
            layer.setShadowRadius(6.0);
            layer.setShadowOpacity(0.0);
            layer.setShadowOffset(NSSize::new(0.0, 0.0));
        }
        vfx.addSubview(&dot);
        processing_dots.push(dot);
    }

    // Animated gradient border — hidden at rest, only shown during Processing.
    let gradient = CAGradientLayer::new();
    gradient.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(LISTEN_W, LISTEN_H),
    ));
    gradient.setType(unsafe { kCAGradientLayerConic });
    gradient.setStartPoint(NSPoint::new(0.5, 0.5));
    gradient.setEndPoint(NSPoint::new(1.0, 0.5));
    set_gradient_colors(&gradient, &theme);
    set_gradient_locations(&gradient);
    gradient.setHidden(true);

    let mask_shape = CAShapeLayer::new();
    mask_shape.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(LISTEN_W, LISTEN_H),
    ));
    mask_shape.setFillColor(Some(&NSColor::clearColor().CGColor()));
    mask_shape.setStrokeColor(Some(&NSColor::blackColor().CGColor()));
    mask_shape.setLineWidth(BORDER_GRADIENT_W);
    unsafe {
        mask_shape.setPath(Some(&rounded_rect_path(
            LISTEN_W,
            LISTEN_H,
            CORNER_RADIUS,
            BORDER_GRADIENT_W,
        )));
        gradient.setMask(Some(&*mask_shape));
    }
    if let Some(cap_layer) = capsule.layer() {
        cap_layer.addSublayer(&gradient);
    }

    capsule.addSubview(&vfx);
    root.addSubview(&capsule);
    panel.setContentView(Some(&root));
    panel.setAlphaValue(0.0);
    panel.orderFrontRegardless();
    PANEL_OPEN.store(true, Ordering::Relaxed);

    ui.panel = Some(panel);
    ui.root_view = Some(root);
    ui.capsule_view = Some(capsule);
    ui.vfx_view = Some(vfx);
    ui.bg_overlay = Some(bg);
    ui.text_label = Some(label);
    ui.listen_indicator = Some(listen_indicator);
    ui.processing_dots = processing_dots;
    ui.gradient_border = Some(gradient);
    ui.gradient_mask = Some(mask_shape);
    ui.screen_center_x = center_x;
    ui.screen_top_y = top_y;
}
