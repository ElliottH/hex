use super::*;
use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
use rune::text::TextSystem;
use rune::{Env, Mods, Runtime};

/// Tall enough to show every section of the fixture without scrolling.
const GOLDEN_SIZE: (f32, f32) = (720.0, 1640.0);
const GOLDEN_SCALE: f32 = 2.0;

fn runtime(scene: &str, dark: bool, size: (f32, f32)) -> Runtime<Ui> {
    let mut state = Ui::with(AppSettings::default(), Service::Fixture(Vec::new()), None);
    let (_, apply) = SCENES
        .iter()
        .find(|(name, _)| *name == scene)
        .unwrap_or_else(|| panic!("no scene {scene}"));
    apply(&mut state);
    let env = Env {
        system_dark: dark,
        scale: GOLDEN_SCALE,
    };
    let text = TextSystem::new(rune_theme_opencode::FONTS, rune_theme_opencode::FAMILIES);
    let mut rt = Runtime::with_text(state, view, size, env, text);
    rt.poll = Some(poll);
    rt.on_key = Some(on_key);
    rt.frame(0.0);
    rt
}

fn sent(rt: &Runtime<Ui>) -> &[Request] {
    match &rt.state.service {
        Service::Fixture(sent) => sent,
        _ => panic!("fixtures never connect"),
    }
}

/// Presses an element by key and lets every animation settle.
fn tap(rt: &mut Runtime<Ui>, key: &str) {
    let (x, y) = rt.find(key).unwrap_or_else(|| panic!("no element {key}"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    for _ in 0..60 {
        rt.frame(1.0 / 60.0);
    }
}

fn key(rt: &mut Runtime<Ui>, key: Key, mods: Mods) {
    rt.key(KeyEvent { key, mods });
    rt.frame(1.0 / 60.0);
}

#[test]
fn portal_desktops_show_what_they_granted_and_keep_recording_rows() {
    let rt = runtime("portal", true, GOLDEN_SIZE);
    let texts: Vec<String> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    for expected in [
        "Desktop shortcuts",
        "F12",
        "Ctrl+Alt+V",
        "Granted by your desktop",
        "Dictation shortcut",
    ] {
        assert!(
            texts.iter().any(|text| text == expected),
            "missing {expected:?} in {texts:?}"
        );
    }
}

#[test]
fn desktops_without_a_portal_list_commands_instead_of_recording_rows() {
    let rt = runtime("commands", true, GOLDEN_SIZE);
    let texts: Vec<String> = rt.elements().into_iter().filter_map(|e| e.text).collect();
    assert!(texts.iter().any(|text| text == "hex dictation toggle"));
    assert!(!texts.iter().any(|text| text == "Dictation shortcut"));
    assert!(rt.find("paste-last-hotkey").is_none());
}

#[test]
fn toggles_save_and_ask_the_service_to_reload() {
    let mut rt = runtime("portal", true, GOLDEN_SIZE);
    assert!(rt.state.settings.paste_with_shift);
    tap(&mut rt, "terminal-paste");
    assert!(!rt.state.settings.paste_with_shift);
    assert!(matches!(sent(&rt), [Request::ReloadSettings]));
}

#[test]
fn enabling_sends_the_listener_action() {
    let mut rt = runtime("disabled", true, GOLDEN_SIZE);
    tap(&mut rt, "toggle-listening");
    assert!(matches!(
        sent(&rt),
        [Request::Desktop(DesktopAction::StartListening)]
    ));
}

#[test]
fn releasing_the_microphone_asks_first() {
    let mut rt = runtime("portal", true, GOLDEN_SIZE);
    tap(&mut rt, "microphone-mode-control/segment1");
    assert!(rt.state.confirm_release);
    assert!(!rt.state.settings.release_microphone_while_idle);
    tap(&mut rt, "confirm-release/actions/confirm-release");
    assert!(rt.state.settings.release_microphone_while_idle);
    assert!(!rt.state.confirm_release);
}

#[test]
fn recording_a_shortcut_sends_the_chord_to_the_service() {
    let mut rt = runtime("portal", true, GOLDEN_SIZE);
    tap(&mut rt, "dictation-hotkey");
    assert!(matches!(
        sent(&rt),
        [Request::CaptureShortcutFor(ShortcutTarget::Dictation)]
    ));
    key(&mut rt, Key::Text("d".into()), Mods::default());
    assert!(matches!(
        rt.state.capture,
        Capture::Listening {
            message: Some("Add a modifier"),
            ..
        }
    ));
    let mods = Mods {
        ctrl: true,
        alt: true,
        cmd: true,
        ..Mods::default()
    };
    key(&mut rt, Key::Command('d'), mods);
    let Some(Request::SetShortcut { target, shortcut }) = sent(&rt).last() else {
        panic!("no shortcut sent: {:?}", sent(&rt));
    };
    assert_eq!(*target, ShortcutTarget::Dictation);
    assert!(shortcut.control && shortcut.alt && !shortcut.shift && !shortcut.platform);
    assert_eq!(shortcut.key, "d");
    assert!(matches!(rt.state.capture, Capture::Saving { .. }));
}

#[test]
fn escape_cancels_a_shortcut_recording() {
    let mut rt = runtime("capturing", true, GOLDEN_SIZE);
    key(&mut rt, Key::Escape, Mods::default());
    assert_eq!(rt.state.capture, Capture::Idle);
    assert!(matches!(sent(&rt), [Request::CancelShortcut]));
}

#[test]
fn function_keys_and_super_record_as_linux_key_names() {
    let f12 = recorded_binding(&KeyEvent {
        key: Key::Function(12),
        mods: Mods::default(),
    })
    .unwrap();
    assert_eq!(desktop_shortcut(&f12).key, "f12");
    let space = recorded_binding(&KeyEvent {
        key: Key::Text(" ".into()),
        mods: Mods {
            logo: true,
            ..Mods::default()
        },
    })
    .unwrap();
    let shortcut = desktop_shortcut(&space);
    assert_eq!(shortcut.key, "space");
    assert!(shortcut.platform);
}

#[test]
fn shortcuts_that_collide_are_refused() {
    let ui = fixture(portal_shortcuts());
    let paste = ui.settings.paste_last_hotkey.clone().unwrap();
    assert_eq!(
        validate(&ui.settings, ShortcutTarget::Dictation, &paste),
        Err("Already in use")
    );
}

#[test]
fn service_settings_are_adopted_only_when_the_service_changes_them() {
    let mut ui = fixture(portal_shortcuts());
    let service = ui.settings.clone();
    ui.adopt(service.clone());
    ui.update(|settings| settings.paste_with_shift = false);
    // The service has not reloaded yet: its stale copy must not win.
    ui.adopt(service.clone());
    assert!(!ui.settings.paste_with_shift);
    let mut changed = service;
    changed.enable_on_launch = false;
    ui.adopt(changed);
    assert!(!ui.settings.enable_on_launch);
}

// ---------------------------------------------------------------------------
// Goldens: offscreen GPU renders compared with checked-in PNGs. Set
// HEX_UPDATE_GOLDENS=1 to rewrite them. Without a GPU adapter they skip.

/// One device for every golden, used one render at a time: creating Vulkan
/// devices from parallel test threads crashed Mesa's RADV on Linux.
fn gpu() -> Option<std::sync::MutexGuard<'static, Gpu>> {
    static GPU: std::sync::OnceLock<Option<std::sync::Mutex<Gpu>>> = std::sync::OnceLock::new();
    GPU.get_or_init(|| {
        std::panic::catch_unwind(|| Gpu::new(Gpu::instance(), None))
            .ok()
            .map(std::sync::Mutex::new)
    })
    .as_ref()
    .map(|gpu| {
        gpu.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    })
}

fn render(scene: &str, dark: bool) -> Option<(u32, u32, Vec<u8>)> {
    let gpu = gpu()?;
    let mut rt = runtime(scene, dark, GOLDEN_SIZE);
    let mut renderer = Renderer::new(&gpu.device, OFFSCREEN_FORMAT);
    let (width, height) = (
        (GOLDEN_SIZE.0 * GOLDEN_SCALE) as u32,
        (GOLDEN_SIZE.1 * GOLDEN_SCALE) as u32,
    );
    let target = Offscreen::new(&gpu.device, width, height);
    renderer.prepare(
        &gpu.device,
        &gpu.queue,
        &mut rt.text.atlas,
        &rt.instances,
        (width, height),
    );
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    renderer.draw(&mut encoder, &target.view, rt.clear_color());
    target.copy(&mut encoder);
    gpu.queue.submit([encoder.finish()]);
    Some((width, height, target.read(&gpu.device)))
}

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("rune-settings-{name}.png"))
}

fn read_png(path: &std::path::Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

fn check_golden(scene: &str, dark: bool) {
    let name = format!("{scene}-{}", if dark { "dark" } else { "light" });
    let Some((width, height, pixels)) = render(scene, dark) else {
        eprintln!("skipping golden {name}: no GPU adapter");
        return;
    };
    let path = golden_path(&name);
    if std::env::var_os("HEX_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        rune::render::write_png(&path, width, height, &pixels).unwrap();
        return;
    }
    assert!(
        path.exists(),
        "{} is missing; run with HEX_UPDATE_GOLDENS=1",
        path.display()
    );
    let (golden_width, golden_height, golden) = read_png(&path);
    assert_eq!(
        (golden_width, golden_height),
        (width, height),
        "{name} size changed"
    );
    // GPU drivers differ in the last bit of blending and glyph coverage;
    // a layout or colour change moves far more than this.
    let differing = pixels
        .chunks(4)
        .zip(golden.chunks(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    let limit = (width * height) as usize / 500;
    if differing > limit {
        let actual = std::env::temp_dir().join(format!("rune-settings-{name}-actual.png"));
        rune::render::write_png(&actual, width, height, &pixels).unwrap();
        panic!(
            "{name}: {differing} pixels differ (limit {limit}); actual render at {}",
            actual.display()
        );
    }
}

#[test]
fn golden_settings_dark() {
    check_golden("portal", true);
}

#[test]
fn golden_settings_light() {
    check_golden("portal", false);
}

#[test]
fn golden_commands_dark() {
    check_golden("commands", true);
}

#[test]
fn golden_menus_and_capture_light() {
    check_golden("menus", false);
}
