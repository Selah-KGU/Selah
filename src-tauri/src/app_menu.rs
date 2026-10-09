use tauri::{
    image::Image,
    menu::{AboutMetadataBuilder, Menu, MenuBuilder, MenuItemBuilder, SubmenuBuilder},
    AppHandle, Manager,
};

const SETTINGS: &str = "app-settings";
const NAV_HOME: &str = "nav-home";
const NAV_TIMETABLE: &str = "nav-timetable";
const NAV_NOTIFICATIONS: &str = "nav-notifications";
const NAV_TODO: &str = "nav-todo";
const NAV_MAIL: &str = "nav-mail";
const STUDY_LIVE: &str = "study-live";
const STUDY_FILES: &str = "study-files";
const STUDY_GRADES: &str = "study-grades";
const STUDY_SYLLABUS: &str = "study-syllabus";
const STUDY_DETECTIVE: &str = "study-detective";
const AGENT_MAIN: &str = "agent-main";
const WINDOW_MAIN: &str = "window-main";
const WINDOW_RELOAD: &str = "window-reload";
const HELP_GUIDE: &str = "help-guide";
const HELP_RELEASES: &str = "help-releases";
const HELP_GITHUB: &str = "help-github";

fn item(
    app: &AppHandle,
    id: &str,
    text: &str,
    accelerator: Option<&str>,
) -> tauri::Result<tauri::menu::MenuItem<tauri::Wry>> {
    let mut builder = MenuItemBuilder::with_id(id, text);
    if let Some(accelerator) = accelerator {
        builder = builder.accelerator(accelerator);
    }
    builder.build(app)
}

pub fn build(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let about = AboutMetadataBuilder::new()
        .name(Some("Selah"))
        .version(Some(app.package_info().version.to_string()))
        .copyright(Some("Copyright © 2026 Selah-KGU"))
        .credits(Some("新月の下で、知性を繋ぐ。すべての関学生に。"))
        .icon(Some(Image::from_bytes(include_bytes!(
            "../icons/icon.png"
        ))?))
        .build();
    let settings = item(app, SETTINGS, "設定…", Some("CmdOrCtrl+,"))?;
    let app_menu = SubmenuBuilder::new(app, "Selah")
        .about_with_text("Selah について", Some(about))
        .separator()
        .item(&settings)
        .separator()
        .quit_with_text("Selah を終了")
        .build()?;

    let home = item(app, NAV_HOME, "ホーム", Some("CmdOrCtrl+1"))?;
    let timetable = item(app, NAV_TIMETABLE, "時間割", Some("CmdOrCtrl+2"))?;
    let notifications = item(app, NAV_NOTIFICATIONS, "お知らせ", Some("CmdOrCtrl+3"))?;
    let todo = item(app, NAV_TODO, "TODO・課題", Some("CmdOrCtrl+4"))?;
    let mail = item(app, NAV_MAIL, "メール", Some("CmdOrCtrl+5"))?;
    let agent = item(app, AGENT_MAIN, "Selah Agent", Some("CmdOrCtrl+Shift+A"))?;
    let navigation_menu = SubmenuBuilder::new(app, "移動")
        .items(&[&home, &timetable, &notifications, &todo, &mail])
        .separator()
        .item(&agent)
        .build()?;

    let live = item(app, STUDY_LIVE, "Live 講義", Some("CmdOrCtrl+Shift+L"))?;
    let files = item(app, STUDY_FILES, "学習ファイル", Some("CmdOrCtrl+Shift+F"))?;
    let grades = item(app, STUDY_GRADES, "成績照会", None)?;
    let syllabus = item(app, STUDY_SYLLABUS, "シラバス検索", None)?;
    let detective = item(app, STUDY_DETECTIVE, "なるほど", Some("CmdOrCtrl+Shift+D"))?;
    let study_menu = SubmenuBuilder::new(app, "学習")
        .items(&[&live, &files])
        .separator()
        .items(&[&grades, &syllabus])
        .separator()
        .item(&detective)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "編集")
        .undo_with_text("取り消す")
        .redo_with_text("やり直す")
        .separator()
        .cut_with_text("切り取り")
        .copy_with_text("コピー")
        .paste_with_text("ペースト")
        .select_all_with_text("すべてを選択")
        .build()?;

    let main_window = item(app, WINDOW_MAIN, "メインウインドウを表示", None)?;
    let reload_window = item(
        app,
        WINDOW_RELOAD,
        "画面を再読み込み",
        Some("CmdOrCtrl+Shift+R"),
    )?;
    let window_menu = SubmenuBuilder::new(app, "ウインドウ")
        .item(&main_window)
        .item(&reload_window)
        .separator()
        .minimize_with_text("しまう")
        .fullscreen_with_text("フルスクリーンにする")
        .close_window_with_text("ウインドウを閉じる")
        .build()?;

    let guide = item(app, HELP_GUIDE, "使い方ガイド", None)?;
    let releases = item(app, HELP_RELEASES, "リリースノート", None)?;
    let github = item(app, HELP_GITHUB, "GitHub", None)?;
    let help_menu = SubmenuBuilder::new(app, "ヘルプ")
        .items(&[&guide, &releases])
        .separator()
        .item(&github)
        .build()?;

    MenuBuilder::new(app)
        .items(&[
            &app_menu,
            &navigation_menu,
            &study_menu,
            &edit_menu,
            &window_menu,
            &help_menu,
        ])
        .build()
}

fn open_help(app: &AppHandle, url: &str, title: &str) {
    if let Err(err) =
        crate::document_tabs::open_external_tab(app, url.to_string(), Some(title.to_string()))
    {
        log::warn!("menu: failed to open {title}: {err}");
    }
}

pub fn handle_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id().as_ref() {
        SETTINGS => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("settings"));
        }
        NAV_HOME => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("home"));
        }
        NAV_TIMETABLE => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("timetable"));
        }
        NAV_NOTIFICATIONS => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("notifications"));
        }
        NAV_TODO => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("todo"));
        }
        NAV_MAIL => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("mail"));
        }
        STUDY_LIVE => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("live"));
        }
        STUDY_FILES => {
            let _ = crate::document_tabs::open_files_tab(app, None, "ファイル".to_string());
        }
        STUDY_GRADES => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("grades"));
        }
        STUDY_SYLLABUS => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("syllabus"));
        }
        STUDY_DETECTIVE => {
            let _ = crate::document_tabs::open_detective_tab(app);
        }
        AGENT_MAIN => {
            let _ = crate::tray::show_main_window_with_tab(app, Some("agent"));
        }
        WINDOW_MAIN => {
            let _ = crate::tray::show_main_window_with_tab(app, None);
        }
        WINDOW_RELOAD => {
            // Native reload works even when WebKit cannot execute JavaScript.
            // The backend recording and transcript state stay alive.
            if let Some(main) = app.get_webview_window("main") {
                log::info!("main WebView reload requested from native menu");
                if let Err(err) = main.reload() {
                    log::warn!("main WebView reload failed: {err}");
                }
            }
        }
        HELP_GUIDE => open_help(
            app,
            "https://github.com/Selah-KGU/Selah/tree/main/docs/guide",
            "使い方ガイド",
        ),
        HELP_RELEASES => open_help(
            app,
            "https://github.com/Selah-KGU/Selah/releases",
            "リリースノート",
        ),
        HELP_GITHUB => open_help(app, "https://github.com/Selah-KGU/Selah", "GitHub"),
        _ => {}
    }
}
