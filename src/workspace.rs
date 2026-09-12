//! 照片工作区的应用状态。
//!
//! 这一层只管理待处理照片的身份、顺序和选择策略，不依赖 GPUI 或缩略图位图。界面层
//! 可以按 [`PhotoId`] 缓存缩略图等展示数据，而预览、导出等工作流只读取这里的领域快照。

use std::path::PathBuf;

use crate::photo::ExifInfo;

/// 队列中照片的稳定身份。
///
/// 身份独立于当前下标，因此删除或重排照片不会让选中状态漂移到相邻项。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct PhotoId(u64);

impl PhotoId {
    /// 供界面创建稳定元素身份使用的数值。
    pub(crate) fn value(self) -> u64 {
        self.0
    }
}

/// 工作区中的一张照片。
#[derive(Clone, Debug)]
pub struct QueuedPhoto {
    id: PhotoId,
    path: PathBuf,
    exif: Option<ExifInfo>,
}

impl QueuedPhoto {
    pub fn id(&self) -> PhotoId {
        self.id
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn exif(&self) -> Option<&ExifInfo> {
        self.exif.as_ref()
    }

    pub fn set_exif(&mut self, exif: Option<ExifInfo>) {
        self.exif = exif;
    }
}

/// 照片队列及其选择策略。
///
/// 此类型不处理文件筛选、缩略图解码或导出；这些都是调用方的工作流与展示职责。
#[derive(Default)]
pub struct PhotoWorkspace {
    photos: Vec<QueuedPhoto>,
    selected: Option<PhotoId>,
    next_photo_id: u64,
}

impl PhotoWorkspace {
    pub fn len(&self) -> usize {
        self.photos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.photos.is_empty()
    }

    pub fn photos(&self) -> &[QueuedPhoto] {
        &self.photos
    }

    pub fn selected_id(&self) -> Option<PhotoId> {
        self.selected
    }

    pub fn selected_photo(&self) -> Option<&QueuedPhoto> {
        let id = self.selected?;
        self.photo(id)
    }

    pub fn photo(&self, id: PhotoId) -> Option<&QueuedPhoto> {
        self.photos.iter().find(|photo| photo.id == id)
    }

    pub fn photo_mut(&mut self, id: PhotoId) -> Option<&mut QueuedPhoto> {
        self.photos.iter_mut().find(|photo| photo.id == id)
    }

    /// 添加一张照片。队列中已有相同路径时返回 `None`。
    pub fn add(&mut self, path: PathBuf) -> Option<PhotoId> {
        if self.photos.iter().any(|photo| photo.path == path) {
            return None;
        }

        let id = PhotoId(self.next_photo_id);
        self.next_photo_id = self.next_photo_id.wrapping_add(1);
        self.photos.push(QueuedPhoto {
            id,
            path,
            exif: None,
        });
        Some(id)
    }

    /// 选择一张仍在队列中的照片。选择未变化或目标不存在时返回 `false`。
    pub fn select(&mut self, id: PhotoId) -> bool {
        if self.selected == Some(id) || self.photo(id).is_none() {
            return false;
        }
        self.selected = Some(id);
        true
    }

    /// 按当前顺序选择一张照片。
    pub fn select_at(&mut self, ix: usize) -> bool {
        let Some(id) = self.photos.get(ix).map(QueuedPhoto::id) else {
            return false;
        };
        self.select(id)
    }

    /// 删除选中照片，并选择原位置的下一张；不存在时选择上一张。
    ///
    /// 返回被删除的身份，调用方可据此清理缩略图等展示缓存。
    pub fn remove_selected(&mut self) -> Option<PhotoId> {
        let id = self.selected?;
        let ix = self.photos.iter().position(|photo| photo.id == id)?;
        self.photos.remove(ix);
        self.selected = self
            .photos
            .get(ix)
            .or_else(|| self.photos.last())
            .map(QueuedPhoto::id);
        Some(id)
    }

    /// 清空队列并返回原有照片数量。
    pub fn clear(&mut self) -> usize {
        let count = self.photos.len();
        self.photos.clear();
        self.selected = None;
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_keeps_selection_near_its_previous_position() {
        let mut workspace = PhotoWorkspace::default();
        let first = workspace.add(PathBuf::from("first.jpg")).unwrap();
        let second = workspace.add(PathBuf::from("second.jpg")).unwrap();
        let third = workspace.add(PathBuf::from("third.jpg")).unwrap();

        assert!(workspace.select(second));
        assert_eq!(workspace.remove_selected(), Some(second));
        assert_eq!(workspace.selected_id(), Some(third));

        assert!(workspace.select(first));
        assert_eq!(workspace.remove_selected(), Some(first));
        assert_eq!(workspace.selected_id(), Some(third));
    }

    #[test]
    fn duplicate_paths_do_not_create_a_second_photo() {
        let mut workspace = PhotoWorkspace::default();
        assert!(workspace.add(PathBuf::from("same.jpg")).is_some());
        assert_eq!(workspace.add(PathBuf::from("same.jpg")), None);
        assert_eq!(workspace.len(), 1);
    }
}
