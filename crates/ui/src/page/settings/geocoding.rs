//! 逆地理编码设置：有效输入自动应用；未完成的字段留在草稿中。
use crate::app::{
    AppView,
    component::field::{description, field, warning},
};
use crate::backend::geocoding::{
    AddressField, DOCUMENTATION_URL, GeocodingProvider, GeocodingSettings, example_response,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::{GroupBox, GroupBoxVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    link::Link,
    radio::RadioGroup,
    v_flex,
};
use gpui_kit::{App, Context, Entity, IntoElement, Subscription, Task, Window, div, prelude::*};

struct FieldDraft {
    id: u64,
    input: Entity<InputState>,
    _observe: Subscription,
}
pub(in crate::app) struct GeocodingControls {
    provider: GeocodingProvider,
    key: Entity<InputState>,
    fields: Vec<FieldDraft>,
    next_id: u64,
    feedback: Option<(String, bool)>,
    save_task: Option<Task<()>>,
    _observe: Subscription,
}
impl GeocodingControls {
    pub(in crate::app) fn new(
        settings: &GeocodingSettings,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) -> Self {
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .default_value(settings.api_key.clone())
                .placeholder("高德 Web 服务 API Key")
        });
        let next_id = settings
            .fields
            .iter()
            .map(|f| f.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        Self {
            provider: settings.provider,
            _observe: cx.subscribe(&key, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_geocoding_save(cx);
                }
            }),
            key,
            fields: settings
                .fields
                .iter()
                .map(|f| Self::field(f.id, &f.paths, window, cx))
                .collect(),
            next_id,
            feedback: None,
            save_task: None,
        }
    }
    fn field(id: u64, value: &str, window: &mut Window, cx: &mut Context<AppView>) -> FieldDraft {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value.to_owned())
                .placeholder("例如 pois.0.name")
        });
        FieldDraft {
            id,
            _observe: cx.subscribe(&input, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.schedule_geocoding_save(cx);
                }
            }),
            input,
        }
    }
    fn value(&self, cx: &App) -> GeocodingSettings {
        GeocodingSettings {
            provider: self.provider,
            api_key: self.key.read(cx).value().trim().to_owned(),
            fields: self
                .fields
                .iter()
                .map(|f| AddressField {
                    id: f.id,
                    paths: f.input.read(cx).value().to_string(),
                })
                .collect(),
        }
    }
}
impl AppView {
    pub(in crate::app) fn sync_geocoding_controls(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next_id = self.settings.geocoding.next_id;
        let mut controls = GeocodingControls::new(&self.backend.params().geocoding, window, cx);
        controls.next_id = controls.next_id.max(next_id);
        self.settings.geocoding = controls;
    }
    fn schedule_geocoding_save(&mut self, cx: &mut Context<Self>) {
        self.settings.geocoding.feedback = None;
        // 等待一小段连续输入结束，避免编辑 Key 时为每个字符请求一次地址。
        self.settings.geocoding.save_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(350))
                .await;
            let _ = this.update(cx, |this, cx| this.apply_geocoding_settings(cx));
        }));
        cx.notify();
    }

    fn apply_geocoding_settings(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.geocoding.value(cx);
        if let Err(error) = settings.validate() {
            self.settings.geocoding.feedback = Some((error.to_string(), true));
            cx.notify();
            return;
        }
        self.settings.geocoding.feedback = None;
        if self.backend.params().geocoding != settings {
            self.backend.params_mut().geocoding = settings;
            self.persist_settings();
            self.settings.geocoding.feedback = self
                .settings_feedback
                .as_ref()
                .map(|message| (message.to_string(), true));
            self.refresh_preview(cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn render_geocoding_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let controls = &self.settings.geocoding;
        let sample = example_response();
        v_flex().id("settings-geocoding").w_full().min_w_0().gap_6()
            .child(GroupBox::new().title("解析服务")
                .child(field("解析方式", RadioGroup::vertical("geocoding-provider")
                    .children(vec!["内置解析（中国省市区）", "高德（详细地址）"])
                    .selected_index(Some(usize::from(controls.provider == GeocodingProvider::Amap)))
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.settings.geocoding.provider = if *ix == 0 { GeocodingProvider::Builtin } else { GeocodingProvider::Amap };
                        this.apply_geocoding_settings(cx);
                    })), cx))
                .child(field("高德 Web 服务 API Key", Input::new(&controls.key).id("geocoding-key").aria_label("高德 Web 服务 API Key").w_full(), cx))
                .child(description("内置解析离线提供中国省市区。高德会接收照片 GPS 坐标和 Key，提供更详细的地址；Key 仅保存在本机设置，不写入照片或水印预设。")))
            .child(v_flex().w_full().min_w_0().gap_4()
                .child(h_flex().w_full().justify_between().gap_3()
                    .child(div().text_sm().child("地址字段"))
                    .child(Button::new("geocoding-add").icon(IconName::Plus).ghost().small()
                        .accessibility_label("添加地址字段").tooltip("添加地址字段")
                        .disabled(controls.fields.len() >= 64)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let controls = &mut this.settings.geocoding;
                            if controls.fields.len() >= 64 { return; }
                            let id = controls.next_id; controls.next_id = controls.next_id.saturating_add(1);
                            controls.fields.push(GeocodingControls::field(id, "", window, cx));
                            this.apply_geocoding_settings(cx);
                        }))))
                .child(description("省略 regeocode 前缀；英文逗号按顺序拼接字段，点号分隔层级，数字从 0 开始选择数组项。模板名根据路径自动生成。"))
                .child(Link::new("amap-geocoding-docs").href(DOCUMENTATION_URL).child("高德字段文档"))
                .children(controls.fields.iter().enumerate().map(|(index, draft)| {
                    let id = draft.id;
                    let value = AddressField { id, paths: draft.input.read(cx).value().to_string() };
                    let preview = match value.token().and_then(|token| Ok((token, value.render(&sample)?))) {
                        Ok((token, rendered)) => v_flex().w_full().gap_1()
                            .child(div().text_sm().child(format!("插入模板：{token}")))
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(format!("示例结果：{}", if rendered.is_empty() { "（示例数据中无此字段）" } else { &rendered })))
                            .into_any_element(),
                        Err(error) => warning(error.to_string(), cx),
                    };
                    GroupBox::new().id(("geocoding-field", id)).outline().w_full().min_w_0()
                        .child(h_flex().w_full().items_start().gap_3()
                            .child(v_flex().flex_1().min_w_0().gap_3()
                                .child(div().text_sm().font_weight(gpui_kit::FontWeight::MEDIUM).child(format!("字段 {}", index + 1)))
                                .child(Input::new(&draft.input).id(("geocoding-path", id)).aria_label(format!("地址字段 {} 路径", index + 1)).w_full())
                                .child(preview))
                            .child(Button::new(("geocoding-remove", id)).icon(IconName::Trash).ghost().small()
                                .accessibility_label(format!("删除地址字段 {}", index + 1)).tooltip("删除字段")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.settings.geocoding.fields.retain(|field| field.id != id);
                                    this.apply_geocoding_settings(cx);
                                }))))
                }))
                .child(description("以上为示例数据，不是当前照片的地址。内置解析仅填充省市区，高德可读取文档中的其他字段。有效修改会自动保存，并同步到水印编辑器的“插入字段”菜单。")))
            .when_some(controls.feedback.as_ref(), |this, (message, _)| this.child(warning(message.clone(), cx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Render, TestAppContext, component::Root, px, size, test::TestWindowExt as _};
    struct Harness {
        app: Entity<AppView>,
        _observe: Subscription,
    }
    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let panel = self.app.update(cx, |app, cx| {
                app.render_geocoding_settings(cx).into_any_element()
            });
            let app = self.app.read(cx);
            let line = &app.text_groups[0].lines[0];
            v_flex()
                .w_full()
                .p_4()
                .gap_2()
                .child(panel)
                .child(app.render_template_field_menu(
                    line.template.clone(),
                    self.app.clone(),
                    line.id,
                ))
        }
    }
    #[gpui_kit::test]
    fn geocoding_form_saves_restarts_validates_and_inserts_configured_fields(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let directory = std::env::temp_dir().join(format!(
            "lumen-geocoding-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("settings.toml");
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let path = path.clone();
            let slot = slot.clone();
            move |window, cx| {
                let app = cx.new(|cx| {
                    let mut app = AppView::new_with_settings_path(Some(path), window, cx);
                    app.logos.assets.clear();
                    app
                });
                slot.borrow_mut().replace(app.clone());
                let harness = cx.new(|cx| Harness {
                    _observe: cx.observe(&app, |_, _, cx| cx.notify()),
                    app,
                });
                Root::new(harness, window, cx)
            }
        });
        let app = slot.borrow().clone().unwrap();
        cx.simulate_resize(size(px(1000.), px(1600.)));
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.within("geocoding-provider").click(1_usize, cx);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert_eq!(
                app.backend.params().geocoding.provider,
                GeocodingProvider::Builtin
            );
            assert!(app.settings.geocoding.feedback.as_ref().unwrap().1);
        });
        cx.update(|window, cx| {
            window.click("geocoding-key", cx);
            window.input("local-test-key", cx);
            window.click("geocoding-add", cx);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        let id = app.read_with(cx, |app, _| {
            app.settings.geocoding.fields.last().unwrap().id
        });
        cx.update(|window, cx| {
            window.click(("geocoding-path", id), cx);
            window.input("pois.0.name", cx);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        app.read_with(cx, |app, _| {
            assert!(
                app.settings.geocoding.feedback.as_ref().unwrap().1,
                "重复模板不能覆盖已保存配置"
            )
        });
        cx.update(|window, cx| {
            window.click(("geocoding-path", id), cx);
            window.press("cmd-a", cx);
            window.input("addressComponent.streetNumber.number", cx);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        let expected = app.read_with(cx, |app, _| {
            assert!(app.settings.geocoding.feedback.is_none());
            assert_eq!(
                app.backend.params().geocoding.provider,
                GeocodingProvider::Amap
            );
            assert!(
                app.backend
                    .params()
                    .geocoding
                    .tokens()
                    .contains(&"{门牌号}".into())
            );
            app.backend.params().geocoding.clone()
        });
        assert_eq!(crate::backend::settings::load_at(&path).geocoding, expected);
        let restored = app.update_in(cx, |_, window, cx| {
            cx.new(|cx| AppView::new_with_settings_path(Some(path.clone()), window, cx))
        });
        restored.read_with(cx, |app, cx| {
            assert_eq!(app.settings.geocoding.value(cx), expected)
        });
        let line_id = app.read_with(cx, |app, _| app.text_groups[0].lines[0].id);
        cx.update(|window, cx| {
            window.click(("template-field-options", line_id), cx);
            window.press("up", cx);
            window.press("enter", cx);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        app.read_with(cx, |app, cx| {
            assert!(
                app.text_groups[0].lines[0]
                    .template
                    .read(cx)
                    .value()
                    .contains("{门牌号}")
            )
        });
        cx.update(|window, cx| {
            window.click(("geocoding-remove", id), cx);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(400));
        cx.run_until_parked();
        assert_eq!(
            crate::backend::settings::load_at(&path)
                .geocoding
                .fields
                .len(),
            3
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
