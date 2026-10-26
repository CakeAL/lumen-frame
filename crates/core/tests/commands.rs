//! 后端独立宿主的关键契约：不创建窗口，不访问用户真实配置。
use lumen_frame_core::{
    Command, ErrorCode, Request, Session, StoragePaths,
    export::ExportScope,
    persistence::settings::AppSettings,
    rotation::Rotation,
    watermark::{GroupAttachment, PaletteTheme, Placement, TextGroup},
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[ctor::ctor(unsafe)]
fn initialize_fonts() {
    lumen_frame_core::initialize_native_text();
}
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "lumen-backend-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../test_images")
        .join(name)
}
fn call(session: &mut Session, command: Value) -> Value {
    session
        .dispatch_json(json!({"command":command}))
        .result
        .unwrap()
        .value()
        .clone()
}
fn imported(session: &mut Session) {
    session
        .execute(Command::ImportPhotos {
            paths: vec![fixture("DSC_4587.jpg"), fixture("ultra_hdr.jpg")],
        })
        .unwrap();
    assert_eq!(session.workspace().len(), 2);
}
#[test]
fn json_operations_preserve_per_photo_content_and_global_environment() {
    let mut session = Session::default();
    imported(&mut session);
    let first = session.workspace().photos()[0].id();
    let second = session.workspace().photos()[1].id();
    session
        .execute(Command::SetCustomText {
            text: "第一张".into(),
        })
        .unwrap();
    session.select(second);
    session
        .execute(Command::SetCustomText {
            text: "第二张".into(),
        })
        .unwrap();
    session.select(first);
    let settings = AppSettings {
        jpeg_quality: 82,
        default_font: "Arial".into(),
        ..Default::default()
    };
    let scratch = Scratch::new();
    session = session.with_storage(StoragePaths::in_directory(&scratch.0));
    session.execute(Command::SaveSettings { settings }).unwrap();
    call(
        &mut session,
        json!({"method":"patch_params","params":{"changes":{
            "shadow_color":[255,255,255], "palette_card":{"enabled":true},
            "shadow_direction":"Down", "shadow_offset":0.015, "shadow_scale":0.96,
            "rotation":"Clockwise90"
        }}}),
    );
    let before = session.params().palette_card.clone();
    call(
        &mut session,
        json!({"method":"patch_params","params":{"changes":{"palette_card":{"show_hex":true}}}}),
    );
    assert_eq!(session.params().palette_card.enabled(), before.enabled());
    call(&mut session, json!({"method":"apply_current_to_all"}));
    session.select(second);
    assert_eq!(session.params().custom_text, "第二张");
    assert_eq!(session.params().quality, 82);
    assert_eq!(session.params().rotation, Rotation::Clockwise90);
    assert_eq!(session.params().shadow_color, [255; 3]);
    assert_eq!(
        session.params().shadow_direction,
        lumen_frame_core::watermark::ShadowDirection::Down
    );
    assert_eq!(session.params().shadow_offset, 0.015);
    assert_eq!(session.params().shadow_scale, 0.96);
    assert_eq!(
        session.watermark_for(first).unwrap().params.custom_text,
        "第一张"
    );
    let colors = session
        .execute(Command::ExtractPalette {
            photo_id: second,
            count: 6,
            theme: PaletteTheme::default(),
        })
        .unwrap();
    assert!(colors.value().is_array());
    assert_eq!(
        lumen_frame_core::persistence::settings::load_at(session.storage().settings().unwrap())
            .jpeg_quality,
        82
    );
}
#[test]
fn invalid_changes_and_stale_requests_leave_state_unchanged() {
    let mut session = Session::default();
    let before = session.current().clone();
    for changes in [
        json!({"shadow_density":4.1}),
        json!({"shadow_offset":-0.101}),
        json!({"shadow_offset":0.101}),
        json!({"shadow_scale":0.49}),
        json!({"shadow_scale":1.21}),
        json!({"shadow_direction":"unknown"}),
        json!({"palette_card":{"unknown":true}}),
    ] {
        assert!(
            session
                .dispatch_json(
                    json!({"command":{"method":"patch_params","params":{"changes":changes}}})
                )
                .result
                .is_err()
        );
        assert_eq!(session.revision(), 0);
        assert_eq!(session.current(), &before);
    }
    let mut a = TextGroup::default();
    let mut b = a.clone();
    a.attachment = Some(GroupAttachment {
        target: 1,
        side: Placement::Right,
        gap: 0.0,
    });
    b.attachment = Some(GroupAttachment {
        target: 0,
        side: Placement::Left,
        gap: 0.0,
    });
    assert!(
        session
            .execute(Command::SetTextGroups { groups: vec![a, b] })
            .is_err()
    );
    assert_eq!(session.current(), &before);
    session
        .execute(Command::SetCustomText {
            text: "已更新".into(),
        })
        .unwrap();
    let reply = session.dispatch(Request {
        expected_revision: Some(0),
        command: Command::ClearPhotos,
    });
    assert!(matches!(
        reply.result.unwrap_err().code,
        ErrorCode::Conflict
    ));
    assert!(matches!(
        session
            .dispatch_json(json!({"command":{"method":"unknown"}}))
            .result
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    ));
}
#[test]
fn rich_text_formats_atomic_tokens_and_tracks_edits() {
    let mut session = Session::default();
    call(
        &mut session,
        json!({"method":"edit_text_line","params":{"group_ix":0,"line_ix":0,"template":"前{型号} 后"}}),
    );
    call(
        &mut session,
        json!({"method":"format_text_line","params":{"group_ix":0,"line_ix":0,"start":4,"end":7,"change":{"kind":"color","value":[12,34,56]}}}),
    );
    let style = &session.current().text_groups[0].text.text_params[0];
    assert_eq!((style.spans[0].start, style.spans[0].end), (3, 11));
    call(
        &mut session,
        json!({"method":"edit_text_line","params":{"group_ix":0,"line_ix":0,"template":"插入前{型号} 后"}}),
    );
    let style = &session.current().text_groups[0].text.text_params[0];
    assert_eq!((style.spans[0].start, style.spans[0].end), (9, 17));
    call(
        &mut session,
        json!({"method":"format_text_line","params":{"group_ix":0,"line_ix":0,"start":0,"end":0,"change":{"kind":"bold","value":true}}}),
    );
    assert!(session.current().text_groups[0].text.text_params[0].bold);
}
#[test]
fn presets_and_removal_use_the_same_attachment_rules_without_a_window() {
    let scratch = Scratch::new();
    let mut session = Session::default().with_storage(StoragePaths::in_directory(&scratch.0));
    let mut groups = vec![TextGroup::default(); 3];
    groups[2].attachment = Some(GroupAttachment {
        target: 1,
        side: Placement::Right,
        gap: 0.01,
    });
    session.execute(Command::SetTextGroups { groups }).unwrap();
    session
        .params_mut()
        .palette_card
        .set_attachment(Some(GroupAttachment {
            target: 2,
            side: Placement::Bottom,
            gap: 0.01,
        }));
    session
        .execute(Command::RemoveTextGroup { group_ix: 0 })
        .unwrap();
    assert_eq!(
        session.current().text_groups[1]
            .attachment
            .as_ref()
            .unwrap()
            .target,
        0
    );
    assert_eq!(
        session.params().palette_card.attachment().unwrap().target,
        1
    );
    session
        .execute(Command::SavePreset {
            name: "边框与色卡".into(),
        })
        .unwrap();
    session
        .execute(Command::SetCustomText {
            text: "当前内容".into(),
        })
        .unwrap();
    session.execute(Command::ResetWatermark).unwrap();
    session
        .execute(Command::LoadPreset {
            name: "边框与色卡".into(),
            all_photos: false,
        })
        .unwrap();
    assert_eq!(session.current().text_groups.len(), 2);
    assert_eq!(session.params().custom_text, "当前内容");
    let saved = session
        .execute(Command::SavePreset {
            name: "../越界".into(),
        })
        .unwrap();
    let path = Path::new(saved.value()["path"].as_str().unwrap());
    assert_eq!(path.parent().unwrap(), session.storage().presets().unwrap());
}
#[test]
fn frozen_jobs_and_headless_preview_preserve_hdr_export() {
    let scratch = Scratch::new();
    let mut session = Session::default().with_storage(StoragePaths::in_directory(&scratch.0));
    session.add_photos(vec![fixture("ultra_hdr.jpg")]);
    let id = session.workspace().selected_id().unwrap();
    session
        .execute(Command::SetEnvironment {
            output_folder: Some(scratch.0.clone()),
            jpeg_quality: 91,
            default_font: "Arial".into(),
            rotation: Rotation::None,
        })
        .unwrap();
    session
        .execute(Command::SetTextGroups { groups: vec![] })
        .unwrap();
    let job = session
        .export_jobs(ExportScope::CurrentPhoto)
        .pop()
        .unwrap();
    session.params_mut().quality = 10;
    assert_eq!(job.watermark.params.quality, 91);
    let preview = scratch.0.join("preview.png");
    session
        .execute(Command::RenderPreview {
            photo_id: id,
            max_edge: 320,
            output: preview.clone(),
        })
        .unwrap();
    assert!(image::open(preview).unwrap().width() > 0);
    job.run().unwrap();
    let output =
        lumen_frame_core::media::load_base_image(&scratch.0.join("ultra_hdr_watermark.jpg"))
            .unwrap();
    assert!(lumen_frame_core::gainmap::get_gainmap(&output).is_some());
}

#[test]
fn logo_commands_share_named_imports_and_delete_with_the_desktop() {
    let scratch = Scratch::new();
    let source = scratch.0.join("logo.svg");
    std::fs::write(&source, br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="40" height="40" fill="#ffca00"/></svg>"##).unwrap();
    let mut session = Session::default().with_storage(StoragePaths::in_directory(&scratch.0));
    let added = call(
        &mut session,
        json!({"method":"import_logos","params":{"paths":[source],"names":["旅行"]}}),
    );
    assert_eq!(added[0]["token"], "{尼康logo}");
    assert_eq!(added[0]["builtin"], true);
    assert_eq!(added[1]["token"], "{旅行logo}");
    assert!(session.params().custom_logos.contains_key("旅行logo"));
    let removed = call(
        &mut session,
        json!({"method":"delete_logo","params":{"logo_id":added[1]["id"]}}),
    );
    assert_eq!(removed.as_array().unwrap().len(), 1);
    assert!(!session.params().custom_logos.contains_key("旅行logo"));
    // 保持旧客户端兼容：省略 names 仍使用默认编号。
    let old_client = call(
        &mut session,
        json!({"method":"import_logos","params":{"paths":[source]}}),
    );
    assert_eq!(old_client[1]["token"], "{自定义logo2}");
}
