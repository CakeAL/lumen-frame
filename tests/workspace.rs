//! 工作区在真实窗口里的行为。
//!
//! 这些用例覆盖「队列 → 选中 → 预览」这条主链路。它跨了实体、订阅和后台任务三种机制，
//! 只看单个函数的单元测试看不出接线有没有真的接上。
//!
//! 走的是 [`AppView`] 的公开命令接口（加入队列、切换选中、移除），和界面上的按钮调用
//! 的是同一批方法，所以这里不会因为内部重构而失效。

use std::path::PathBuf;
use std::time::Duration;
use std::{cell::RefCell, rc::Rc};

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Entity, ExternalPaths, FileDropEvent, InputEvent as _, ScrollDelta,
    TestAppContext, VisualTestContext, point, px,
};

use lumen_frame::ui::{AppPage, AppView};

const PHOTO: &str = "./test_images/DSC_4587.jpg";
const OTHER_PHOTO: &str = "./test_images/ultra_hdr.jpg";

fn workspace(cx: &mut TestAppContext) -> (Entity<AppView>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::component::set_locale("zh-CN");
    });
    let view = Rc::new(RefCell::new(None));
    let (_, cx) = cx.add_window_view({
        let view_slot = view.clone();
        move |window, cx| {
            let app = cx.new(|cx| AppView::new(window, cx));
            view_slot.borrow_mut().replace(app.clone());
            Root::new(app, window, cx)
        }
    });
    let app = view.borrow().clone().expect("测试窗口没有创建 AppView");
    (app, cx)
}

/// 预览走后台线程，而且要越过防抖窗口，所以得推着调度器的时钟往前走。
fn settle(cx: &mut VisualTestContext, ready: impl Fn(&mut VisualTestContext) -> bool) {
    for _ in 0..40 {
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        if ready(cx) {
            return;
        }
    }
    panic!("等待预览渲染完成超时");
}

#[gpui_kit::test]
fn dropped_photos_reach_the_queue_and_the_preview(cx: &mut TestAppContext) {
    let (view, cx) = workspace(cx);

    assert_eq!(view.read_with(cx, |view, _| view.photo_count()), 0);
    assert!(
        view.read_with(cx, |view, cx| view.preview_image(cx))
            .is_none()
    );

    view.update_in(cx, |view, _, cx| {
        view.add_photos(
            vec![
                PathBuf::from(PHOTO),
                PathBuf::from("./test_images/readme.txt"),
            ],
            cx,
        );
    });

    assert_eq!(
        view.read_with(cx, |view, _| view.photo_count()),
        1,
        "非图片文件不应该进入队列"
    );
    assert_eq!(
        view.read_with(cx, |view, _| view
            .selected_path()
            .map(|path| path.to_path_buf())),
        Some(PathBuf::from(PHOTO))
    );

    settle(cx, |cx| {
        view.read_with(cx, |view, cx| view.preview_image(cx))
            .is_some()
    });

    let image = view
        .read_with(cx, |view, cx| view.preview_image(cx))
        .expect("队列里有照片，却没有渲染出预览");
    let size = image.size(0);
    assert!(size.width.0 > 0 && size.height.0 > 0);
    assert_eq!(image.frame_count(), 1);

    // 同一个文件拖两次不应该排出两张卡片。
    view.update_in(cx, |view, _, cx| {
        view.add_photos(vec![PathBuf::from(PHOTO)], cx);
    });
    assert_eq!(view.read_with(cx, |view, _| view.photo_count()), 1);
}

#[gpui_kit::test]
fn switching_photos_recomputes_the_preview(cx: &mut TestAppContext) {
    let (view, cx) = workspace(cx);

    view.update_in(cx, |view, _, cx| {
        view.add_photos(vec![PathBuf::from(PHOTO), PathBuf::from(OTHER_PHOTO)], cx);
    });
    assert_eq!(view.read_with(cx, |view, _| view.photo_count()), 2);

    settle(cx, |cx| {
        view.read_with(cx, |view, cx| view.preview_image(cx))
            .is_some()
    });
    let first = view
        .read_with(cx, |view, cx| view.preview_image(cx))
        .expect("第一张照片没有渲染出预览");

    // 换到第二张：预览必须换成另一张位图，而不是继续显示上一张。
    view.update_in(cx, |view, _, cx| view.select_photo_at(1, cx));
    settle(cx, |cx| {
        view.read_with(cx, |view, cx| {
            view.preview_image(cx)
                .is_some_and(|image| image.id != first.id)
        })
    });
}

#[gpui_kit::test]
fn removing_the_last_photo_empties_the_preview(cx: &mut TestAppContext) {
    let (view, cx) = workspace(cx);

    view.update_in(cx, |view, _, cx| {
        view.add_photos(vec![PathBuf::from(PHOTO)], cx);
    });
    settle(cx, |cx| {
        view.read_with(cx, |view, cx| view.preview_image(cx))
            .is_some()
    });

    view.update_in(cx, |view, _, cx| view.remove_selected(cx));

    assert_eq!(view.read_with(cx, |view, _| view.photo_count()), 0);
    assert_eq!(
        view.read_with(cx, |view, _| view
            .selected_path()
            .map(|path| path.to_path_buf())),
        None
    );
    assert!(
        view.read_with(cx, |view, cx| view.preview_image(cx))
            .is_none(),
        "队列空了以后预览应该回到空状态"
    );
}

/// 把文件拖到工作区上，是队列最主要的入口，所以它值得一条端到端的用例：
/// 走的是平台投递的拖放事件，而不是直接调用 `add_photos`。
#[gpui_kit::test]
fn dropping_files_on_the_workspace_enqueues_them(cx: &mut TestAppContext) {
    let (view, cx) = workspace(cx);

    // 先让工作区渲染出一帧，命中测试才有可用的边界。
    cx.run_until_parked();

    // 窗口在测试里是最大化的；这个位置落在中间的预览区里，属于工作区的拖放目标。
    let position = point(px(420.), px(320.));
    cx.update(|window, cx| {
        window.dispatch_event(
            FileDropEvent::Entered {
                position,
                paths: ExternalPaths([PathBuf::from(PHOTO)].into_iter().collect()),
            }
            .to_platform_input(),
            cx,
        );
        window.dispatch_event(FileDropEvent::Submit { position }.to_platform_input(), cx);
    });
    cx.run_until_parked();

    assert_eq!(
        view.read_with(cx, |view, _| view.photo_count()),
        1,
        "拖入的文件没有进入队列"
    );
    settle(cx, |cx| {
        view.read_with(cx, |view, cx| view.preview_image(cx))
            .is_some()
    });
}

/// 设置页要能渲染，而且切过去再切回来不能出问题。
///
/// 明暗选择是三项单选，索引和 `AppearanceMode` 的对应关系错了会在渲染时暴露出来；
/// 默认值本身由 `tests/settings.rs` 覆盖，这里不依赖开发机上的偏好文件。
#[gpui_kit::test]
fn settings_page_renders_and_returns(cx: &mut TestAppContext) {
    let (view, cx) = workspace(cx);

    view.update_in(cx, |view, _, cx| view.go_to(AppPage::Settings, cx));
    cx.run_until_parked();

    view.update_in(cx, |view, _, cx| view.go_to(AppPage::Watermark, cx));
    cx.run_until_parked();

    // 切回来之后主界面仍然可用。
    assert_eq!(view.read_with(cx, |view, _| view.photo_count()), 0);
}

/// 文字参数入口保持在检查器里，详细设置另开窗口，不能遮挡主窗口的照片。
#[gpui_kit::test]
fn text_group_editor_opens_in_a_separate_window(cx: &mut TestAppContext) {
    let (view, cx) = workspace(cx);
    cx.run_until_parked();

    cx.update(|window, cx| {
        window.scroll(
            "solid-background",
            ScrollDelta::Pixels(point(px(0.), px(-2_000.))),
            cx,
        );
        window.render_frame(cx);
        window.click(("text-group-edit", 0_u64), cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
    });
    assert_eq!(
        view.read_with(cx, |view, _| view.text_editor_window_count()),
        1
    );
}

/// 新建文字组会得到独立状态，并直接打开该组的独立编辑窗口。
#[gpui_kit::test]
fn adding_a_text_group_opens_its_editor(cx: &mut TestAppContext) {
    let (view, cx) = workspace(cx);
    cx.run_until_parked();

    assert_eq!(view.read_with(cx, |view, _| view.text_group_count()), 1);
    cx.update(|window, cx| {
        window.scroll(
            "solid-background",
            ScrollDelta::Pixels(point(px(0.), px(-2_000.))),
            cx,
        );
        window.render_frame(cx);
        window.click("text-group-add", cx);
        window.render_frame(cx);
        assert!(window.try_find("dialog").is_none());
    });
    assert_eq!(view.read_with(cx, |view, _| view.text_group_count()), 2);
    assert_eq!(
        view.read_with(cx, |view, _| view.text_editor_window_count()),
        1
    );
}
