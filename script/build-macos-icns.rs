//! 将标准 `.iconset` 目录封装为现代 PNG-backed ICNS。
//!
//! macOS 27 测试版的 `iconutil` 无法重新打包它自己解出的 iconset，因此打包脚本使用
//! 已生成的 `LumenFrame.icns`；需要更新图标时可以用这个无依赖小工具重新封装。

use std::{convert::TryInto as _, env, fs, io::Write as _, path::Path};

const ENTRIES: &[(&str, &[u8; 4])] = &[
    ("icon_16x16.png", b"icp4"),
    ("icon_16x16@2x.png", b"icp5"),
    ("icon_32x32@2x.png", b"icp6"),
    ("icon_128x128.png", b"ic07"),
    ("icon_128x128@2x.png", b"ic08"),
    ("icon_256x256@2x.png", b"ic09"),
    ("icon_512x512@2x.png", b"ic10"),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let iconset = args.next().ok_or("缺少 iconset 目录")?;
    let output = args.next().ok_or("缺少输出 ICNS 路径")?;
    if args.next().is_some() {
        return Err("参数过多".into());
    }

    let iconset = Path::new(&iconset);
    let chunks = ENTRIES
        .iter()
        .map(|(name, kind)| Ok((*kind, fs::read(iconset.join(name))?)))
        .collect::<Result<Vec<_>, std::io::Error>>()?;
    let total_size = 8usize
        + chunks
            .iter()
            .map(|(_, png)| 8usize + png.len())
            .sum::<usize>();
    let total_size: u32 = total_size.try_into()?;

    let mut file = fs::File::create(output)?;
    file.write_all(b"icns")?;
    file.write_all(&total_size.to_be_bytes())?;
    for (kind, png) in chunks {
        let chunk_size: u32 = (png.len() + 8).try_into()?;
        file.write_all(kind)?;
        file.write_all(&chunk_size.to_be_bytes())?;
        file.write_all(&png)?;
    }
    Ok(())
}
