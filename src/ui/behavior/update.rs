//! 应用更新的 UI 生命周期与后台任务编排。

use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::{AppContext as _, Context, SharedString, Window};

use crate::update::{self, UpdateAvailability};

use super::super::AppView;

#[derive(Clone, Debug)]
pub(in crate::ui::app) enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available { version: SharedString },
    Installing { version: SharedString },
    Installed { version: SharedString },
    Failed { message: SharedString },
}

impl AppView {
    pub(in crate::ui::app) fn start_automatic_update_check(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if update::automatic_checks_enabled() {
            self.start_update_check(true, window, cx);
        }
    }

    pub(in crate::ui::app) fn check_for_updates(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.update_state.clone() {
            UpdateState::Available { version } => self.confirm_update_install(version, window, cx),
            UpdateState::Checking | UpdateState::Installing { .. } => {}
            _ => self.start_update_check(false, window, cx),
        }
    }

    fn start_update_check(&mut self, automatic: bool, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.update_state,
            UpdateState::Checking | UpdateState::Installing { .. }
        ) {
            return;
        }
        self.update_state = UpdateState::Checking;
        cx.notify();

        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if automatic {
                        update::check_if_due()
                    } else {
                        update::check_now_and_record().map(Some)
                    }
                })
                .await;

            let _ = this.update(cx, |this, cx| match result {
                Ok(None) => {
                    this.update_state = UpdateState::Idle;
                    cx.notify();
                }
                Ok(Some(UpdateAvailability::UpToDate)) => {
                    this.update_state = UpdateState::UpToDate;
                    cx.notify();
                }
                Ok(Some(UpdateAvailability::Available { version })) => {
                    let version = SharedString::from(version);
                    this.update_state = UpdateState::Available {
                        version: version.clone(),
                    };
                    if automatic {
                        let _ = window_handle.update(cx, |_, window, cx| {
                            window.push_notification(
                                Notification::success(format!(
                                    "发现新版本 {version}，可在“设置 > 关于”中安装"
                                )),
                                cx,
                            )
                        });
                    }
                    cx.notify();
                }
                Err(error) => {
                    let message = format!("检查更新失败：{error:#}");
                    this.update_state = UpdateState::Failed {
                        message: message.clone().into(),
                    };
                    if !automatic {
                        let _ = window_handle.update(cx, |_, window, cx| {
                            window.push_notification(Notification::error(message), cx)
                        });
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn confirm_update_install(
        &mut self,
        version: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let version_for_install = version.clone();
            alert
                .title(format!("发现新版本 {version}"))
                .description(
                    "更新将从 GitHub 下载并替换当前应用。安装完成后请重新启动 Lumen Frame。",
                )
                .button_props(
                    gpui_kit::component::dialog::DialogButtonProps::default()
                        .ok_text("安装更新")
                        .show_cancel(true)
                        .cancel_text("稍后")
                        .on_ok(move |_, window, cx| {
                            let version = version_for_install.clone();
                            view.update(cx, |this, cx| this.install_update(version, window, cx));
                            true
                        }),
                )
        });
    }

    fn install_update(
        &mut self,
        version: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.update_state, UpdateState::Installing { .. }) {
            return;
        }
        self.update_state = UpdateState::Installing {
            version: version.clone(),
        };
        cx.notify();

        let window_handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { update::install_latest() })
                .await;
            let _ = this.update(cx, |this, cx| {
                let notification = match result {
                    Ok(installed_version) => {
                        this.update_state = UpdateState::Installed {
                            version: installed_version.clone().into(),
                        };
                        Notification::success(format!(
                            "已安装 {installed_version}，重新启动应用后生效"
                        ))
                    }
                    Err(error) => {
                        let message = format!("安装更新失败：{error:#}");
                        this.update_state = UpdateState::Failed {
                            message: message.clone().into(),
                        };
                        Notification::error(message)
                    }
                };
                let _ = window_handle.update(cx, |_, window, cx| {
                    window.push_notification(notification, cx)
                });
                cx.notify();
            });
        })
        .detach();
    }
}
