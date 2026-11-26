//! 左侧预设面板的状态、目录排序与增删载入工作流。

use std::collections::{HashMap, HashSet};

use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, SharedString, Window};

use crate::backend::presets::{self, WatermarkPreset};

use super::super::AppView;
use super::super::component::inspector::preset_of;

/// 预设的应用范围；手动画面调整只修改当前照片，导出旋转对全部照片生效。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum PresetScope {
    CurrentPhoto,
    AllPhotos,
}

/// 后台扫描生成的轻量预设目录快照。
pub(in crate::app) struct PresetCatalog {
    pub names: Vec<SharedString>,
    pub previews: HashMap<SharedString, WatermarkPreset>,
    pub builtins: HashSet<SharedString>,
}

/// 左侧预设面板与卡片目录的唯一 UI 状态持有者。
pub(in crate::app) struct PresetPanelState {
    pub scope: PresetScope,
    pub collapsed: bool,
    pub names: Vec<SharedString>,
    /// 拖动落点的预设名与前/后位置，不写入配置。
    pub drop_target: Option<(SharedString, bool)>,
    /// 预设卡片使用的轻量视觉快照，避免在每一帧渲染时读取磁盘。
    pub previews: HashMap<SharedString, WatermarkPreset>,
    /// 编译进应用的预设名称；用于禁止覆盖与删除，并在卡片上标明来源。
    pub builtins: HashSet<SharedString>,
    pub feedback: Option<SharedString>,
    pub feedback_is_error: bool,
}

impl PresetPanelState {
    pub fn new(order: &[String]) -> (Self, Vec<String>) {
        let catalog = load_catalog(order);
        let user_order = user_preset_order(&catalog.names, &catalog.builtins);
        (
            Self {
                scope: PresetScope::AllPhotos,
                collapsed: false,
                names: catalog.names,
                drop_target: None,
                previews: catalog.previews,
                builtins: catalog.builtins,
                feedback: None,
                feedback_is_error: false,
            },
            user_order,
        )
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins.contains(name)
    }

    pub fn clear_feedback(&mut self) {
        self.feedback = None;
    }

    pub fn set_status(&mut self, message: impl Into<SharedString>) {
        self.feedback = Some(message.into());
        self.feedback_is_error = false;
    }

    pub fn set_error(&mut self, message: impl Into<SharedString>) {
        self.feedback = Some(message.into());
        self.feedback_is_error = true;
    }

    pub fn user_order(&self) -> Vec<String> {
        user_preset_order(&self.names, &self.builtins)
    }

    pub fn apply_catalog(&mut self, mut catalog: PresetCatalog, order: &[String]) {
        catalog.names.sort_by_key(|name| {
            if catalog.builtins.contains(name) {
                (0, 0)
            } else {
                (
                    1,
                    order
                        .iter()
                        .position(|saved| saved == name.as_ref())
                        .unwrap_or(usize::MAX),
                )
            }
        });
        self.names = catalog.names;
        self.previews = catalog.previews;
        self.builtins = catalog.builtins;
    }

    pub fn refresh(&mut self, order: &[String]) -> Vec<String> {
        let catalog = load_catalog(order);
        let next_order = user_preset_order(&catalog.names, &catalog.builtins);
        self.names = catalog.names;
        self.previews = catalog.previews;
        self.builtins = catalog.builtins;
        next_order
    }
}

pub(in crate::app) fn load_catalog(order: &[String]) -> PresetCatalog {
    let builtins = presets::builtin().unwrap_or_default();
    let builtin_names = builtins
        .iter()
        .map(|(name, _)| SharedString::from(name.clone()))
        .collect::<HashSet<_>>();
    let mut names = Vec::new();
    let mut previews = HashMap::new();

    for (name, preset) in builtins {
        let name = SharedString::from(name);
        names.push(name.clone());
        previews.insert(name, preset);
    }

    let mut user_names = presets::list().unwrap_or_default();
    sort_preset_names(&mut user_names, order);
    for name in user_names {
        let name = SharedString::from(name);
        if builtin_names.contains(&name) {
            continue;
        }
        if let Ok(preset) = presets::load(&name) {
            names.push(name.clone());
            previews.insert(name, preset);
        }
    }

    PresetCatalog {
        names,
        previews,
        builtins: builtin_names,
    }
}

fn user_preset_order(names: &[SharedString], builtins: &HashSet<SharedString>) -> Vec<String> {
    names
        .iter()
        .filter(|name| !builtins.contains(*name))
        .map(ToString::to_string)
        .collect()
}

/// 稳定排序让未保存过顺序的新项留在末尾，并保留目录原有的字母顺序。
fn sort_preset_names(names: &mut [String], order: &[String]) {
    names.sort_by_key(|name| {
        order
            .iter()
            .position(|saved| saved == name)
            .unwrap_or(usize::MAX)
    });
}

fn move_preset_name(
    names: &mut Vec<SharedString>,
    source: &str,
    target: &str,
    before: bool,
) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = names.iter().position(|name| name.as_ref() == source) else {
        return false;
    };
    let Some(target_index) = names.iter().position(|name| name.as_ref() == target) else {
        return false;
    };
    let slot = target_index + usize::from(!before);
    let destination = slot - usize::from(source_index < slot);
    if destination == source_index {
        return false;
    }
    let name = names.remove(source_index);
    names.insert(destination, name);
    true
}

impl AppView {
    pub(in crate::app) fn toggle_preset_panel(&mut self, cx: &mut Context<Self>) {
        self.presets.collapsed = !self.presets.collapsed;
        cx.notify();
    }

    pub(in crate::app) fn set_builtin_presets_collapsed(
        &mut self,
        collapsed: bool,
        cx: &mut Context<Self>,
    ) {
        if self.preferences.values.builtin_presets_collapsed == collapsed {
            return;
        }
        self.preferences.values.builtin_presets_collapsed = collapsed;
        self.presets.clear_feedback();
        self.persist_preset_preferences();
        cx.notify();
    }

    /// 按名字移动预设，避免拖动过程中目录变化让下标指向另一项。
    pub(in crate::app) fn move_preset(
        &mut self,
        source: &str,
        target: &str,
        before: bool,
        cx: &mut Context<Self>,
    ) {
        self.presets.drop_target = None;
        if self.presets.is_builtin(source)
            || self.presets.is_builtin(target)
            || !move_preset_name(&mut self.presets.names, source, target, before)
        {
            cx.notify();
            return;
        }
        self.preferences.values.preset_order = self.presets.user_order();
        self.presets.clear_feedback();
        self.persist_preset_preferences();
        cx.notify();
    }

    pub(in crate::app) fn move_preset_by(
        &mut self,
        name: &str,
        earlier: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .preferences
            .values
            .preset_order
            .iter()
            .position(|candidate| candidate == name)
        else {
            return;
        };
        let target_index = if earlier {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        if let Some(target) = target_index
            .and_then(|index| self.preferences.values.preset_order.get(index))
            .cloned()
        {
            self.move_preset(name, &target, earlier, cx);
        }
    }

    fn persist_preset_preferences(&mut self) {
        self.persist_settings();
        if let Some(error) = self.preferences.feedback.clone() {
            self.presets.set_error(error);
        }
    }

    /// 把当前配置按输入框里的名字保存成预设。
    pub(in crate::app) fn save_preset(&mut self, cx: &mut Context<Self>) {
        let name = self
            .controls
            .preset_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            return;
        }

        let preset = preset_of(self.backend.params(), &self.build_text_groups(cx));
        match presets::save(&name, &preset) {
            Ok(path) => {
                self.presets
                    .set_status(format!("已保存到 {}", path.display()));
                self.refresh_preset_names();
                self.persist_preset_preferences();
            }
            Err(error) => {
                self.presets.set_error(format!("{error:#}"));
            }
        }
        cx.notify();
    }

    /// 载入预设，并把所有「自己存值」的控件同步到新配置上。
    pub(in crate::app) fn load_preset(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preset = if self.presets.is_builtin(name) {
            presets::load_builtin(name)
        } else {
            presets::load(name)
        };
        match preset {
            Ok(preset) => {
                let all = self.presets.scope == PresetScope::AllPhotos;
                self.apply_preset(preset, window, cx);
                self.presets.set_status(if all {
                    format!("已将「{name}」应用到全部照片及后续导入")
                } else if self.selected_photo_id().is_some() {
                    format!("已将「{name}」应用到当前照片")
                } else {
                    format!("新照片将使用「{name}」")
                });
            }
            Err(error) => {
                self.presets.set_error(format!("{error:#}"));
            }
        }
        cx.notify();
    }

    /// 用一份预设替换当前配置。
    fn apply_preset(
        &mut self,
        preset: WatermarkPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.backend.commit_groups(self.build_text_groups(cx));
        self.backend
            .apply_preset(preset, self.presets.scope == PresetScope::AllPhotos);
        self.restore_selected_watermark(window, cx);
    }

    /// 弹一个确认框再删预设：文件删掉就找不回来了。
    pub(in crate::app) fn confirm_delete_preset(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.presets.is_builtin(name) {
            self.presets.set_error("内置预设不可删除");
            cx.notify();
            return;
        }
        let view = cx.entity();
        let target = name.to_string();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let target = target.clone();
            alert
                .title(format!("删除预设「{target}」？"))
                .description("预设文件会从磁盘上删除，无法恢复。")
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("删除")
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                        .on_ok(move |_, _, cx| {
                            let target = target.clone();
                            view.update(cx, |this, cx| this.delete_preset(&target, cx));
                            true
                        }),
                )
        });
    }

    fn delete_preset(&mut self, name: &str, cx: &mut Context<Self>) {
        match presets::delete(name) {
            Ok(()) => {
                self.presets.set_status(format!("已删除「{name}」"));
                self.refresh_preset_names();
                self.persist_preset_preferences();
            }
            Err(error) => {
                self.presets.set_error(format!("{error:#}"));
            }
        }
        cx.notify();
    }

    /// 确保预设目录存在后交给系统文件管理器显示；空目录也应可直接打开，方便手工管理。
    pub(in crate::app) fn open_preset_folder(&mut self, cx: &mut Context<Self>) {
        let Some(dir) = presets::directory() else {
            self.presets.set_error("找不到系统的配置目录");
            cx.notify();
            return;
        };
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                cx.reveal_path(&dir);
                self.presets.clear_feedback();
            }
            Err(error) => {
                self.presets
                    .set_error(format!("无法打开预设文件夹：{error}"));
            }
        }
        cx.notify();
    }

    fn refresh_preset_names(&mut self) {
        self.preferences.values.preset_order =
            self.presets.refresh(&self.preferences.values.preset_order);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::settings as settings_store;
    use crate::backend::watermark::WatermarkParams;
    use gpui_kit::component::Root;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Entity, TestAppContext, VisualTestContext, point, px};
    use std::{cell::RefCell, path::PathBuf, rc::Rc};

    fn workspace<'a>(
        tag: &str,
        cx: &'a mut TestAppContext,
    ) -> (Entity<AppView>, &'a mut VisualTestContext, PathBuf) {
        let path = std::env::temp_dir()
            .join(format!(
                "lumen-frame-preset-ui-{}-{tag}",
                std::process::id()
            ))
            .join("settings.toml");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        cx.update(gpui_kit::init);
        let view = Rc::new(RefCell::new(None));
        let (_, cx) = cx.add_window_view({
            let view_slot = view.clone();
            let path = path.clone();
            move |window, cx| {
                let app = cx.new(|cx| AppView::new_with_settings_path(Some(path), window, cx));
                view_slot.borrow_mut().replace(app.clone());
                Root::new(app, window, cx)
            }
        });
        let app = view.borrow().clone().unwrap();
        app.update_in(cx, |app, _, cx| {
            // 只替换目录展示数据，不写入用户的预设目录。
            app.presets
                .names
                .retain(|name| app.presets.builtins.contains(name));
            for name in ["test-a", "test-b", "test-c"] {
                app.presets.names.push(name.into());
                app.presets
                    .previews
                    .insert(name.into(), preset_of(&WatermarkParams::default(), &[]));
            }
            app.preferences.values.preset_order = app.presets.user_order();
            cx.notify();
        });
        cx.run_until_parked();
        (app, cx, path)
    }

    #[test]
    fn saved_order_survives_removed_and_new_presets() {
        let mut names = vec![
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "new".to_owned(),
        ];
        sort_preset_names(
            &mut names,
            &["gone".into(), "c".into(), "a".into(), "c".into()],
        );
        assert_eq!(names, ["c", "a", "b", "new"]);
    }

    #[gpui_kit::test]
    fn builtin_group_can_collapse_and_remember_its_state(cx: &mut TestAppContext) {
        let (view, cx, path) = workspace("collapse", cx);
        assert!(cx.debug_bounds("preset-card-16_9").is_some());
        cx.update(|window, cx| {
            window.click("builtin-presets-toggle", cx);
            window.render_frame(cx);
        });
        assert!(view.read_with(cx, |app, _| {
            app.preferences.values.builtin_presets_collapsed
        }));
        assert!(cx.debug_bounds("preset-card-16_9").is_none());
        assert!(cx.debug_bounds("preset-card-test-a").is_some());
        assert!(settings_store::load_at(&path).builtin_presets_collapsed);

        // 折叠入口保持键盘可操作。
        cx.update(|window, cx| window.press("enter", cx));
        assert!(!view.read_with(cx, |app, _| {
            app.preferences.values.builtin_presets_collapsed
        }));
        assert!(cx.debug_bounds("preset-card-16_9").is_some());
        assert!(!settings_store::load_at(&path).builtin_presets_collapsed);
        cx.update(|window, cx| window.press("space", cx));
        cx.update(|window, cx| {
            let restored =
                cx.new(|cx| AppView::new_with_settings_path(Some(path.clone()), window, cx));
            assert!(
                restored
                    .read(cx)
                    .preferences
                    .values
                    .builtin_presets_collapsed
            );
        });
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[gpui_kit::test]
    fn dragging_personal_presets_reorders_and_saves_without_loading(cx: &mut TestAppContext) {
        let (view, cx, path) = workspace("reorder", cx);
        view.update_in(cx, |app, _, cx| app.set_builtin_presets_collapsed(true, cx));
        cx.run_until_parked();
        let source = cx.debug_bounds("preset-card-test-a").unwrap();
        let target = cx.debug_bounds("preset-card-test-c").unwrap();
        cx.update(|window, cx| {
            window.drag(
                source.center(),
                point(target.center().x, target.bottom() - px(8.0)),
                cx,
            );
            window.render_frame(cx);
        });
        let order = view.read_with(cx, |app, _| {
            assert!(app.presets.feedback.is_none(), "拖动不应触发载入预设");
            app.preferences.values.preset_order.clone()
        });
        assert_eq!(order, ["test-b", "test-c", "test-a"]);
        assert_eq!(settings_store::load_at(&path).preset_order, order);

        // 向前拖动插入到目标上方。
        let source = cx.debug_bounds("preset-card-test-a").unwrap();
        let target = cx.debug_bounds("preset-card-test-b").unwrap();
        cx.update(|window, cx| {
            window.drag(
                source.center(),
                point(target.center().x, target.top() + px(8.0)),
                cx,
            );
            window.render_frame(cx);
        });
        assert_eq!(
            settings_store::load_at(&path).preset_order,
            ["test-a", "test-b", "test-c"]
        );

        // 拖动后卡片保持焦点，可直接用键盘再移动一项。
        cx.update(|window, cx| window.press("alt-down", cx));
        assert_eq!(
            settings_store::load_at(&path).preset_order,
            ["test-b", "test-a", "test-c"]
        );
        let names_before = view.read_with(cx, |app, _| app.presets.names.clone());
        view.update_in(cx, |app, _, cx| app.move_preset("test-a", "16_9", true, cx));
        assert_eq!(
            view.read_with(cx, |app, _| app.presets.names.clone()),
            names_before
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
