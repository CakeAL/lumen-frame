# 架构与模块边界

项目使用 Cargo workspace。根包是桌面应用壳，前端和后端是两个可独立编译的库 crate。

## 依赖方向

```text
lumen-frame（src/main.rs：平台初始化、窗口生命周期、应用组装）
  ├── lumen-frame-ui（GPUI 页面、控件、任务调度、纹理）
  │     └── lumen-frame-core
  ├── lumen-frame-mcp（协议、工具 schema、stdio 与本地 IPC）
  │     └── lumen-frame-core
  └── lumen-frame-core（会话、命令、模型、渲染、文件与媒体操作）
```

后端不依赖前端，也不持有 GPUI 窗口、控件实体或 GPU 位图。GPUI 平台字体接口仍是后端的
原生文字渲染适配器，后端的 `gpui-kit` 关闭组件和图标素材默认 feature。独立宿主在主线程
调用 `initialize_native_text()`，桌面宿主注入与窗口共用的字体接口。

## 目录结构

```text
src/
├── main.rs                     # 应用启动、资源注册、窗口生命周期
└── options.rs                  # 桌面 / stdio 启动选项
crates/
├── core/                           # lumen-frame-core
│   ├── Cargo.toml
│   ├── examples/commands.rs        # 无窗口 JSON 命令宿主示例
│   ├── tests/commands.rs           # 跨宿主业务契约
│   └── src/
│       ├── lib.rs                  # 核心库公开入口
│       ├── session.rs              # 队列、当前参数、逐照片快照、默认值
│       ├── command.rs              # 可序列化命令、结果、版本冲突与输入校验
│       ├── import.rs / export.rs / metadata.rs / text.rs
│       ├── image.rs                # RGBA 预览、缩略图与工具媒体操作
│       ├── image/layers.rs         # RGBA 图层缓存、排版与延迟合成
│       ├── watermark/              # 水印输入模型和富文本样式规则
│       ├── workspace.rs            # PhotoId 与队列选择规则
│       ├── photo/                  # 共享照片合成、图层排版和导出
│       ├── render/                 # 画布、阴影、文字、SVG 色卡等渲染
│       ├── media/                  # libvips 生命周期、解码与 EXIF
│       ├── persistence/            # 设置、预设与 Logo 素材读写
│       ├── features/               # 色卡、定位、Motion Photo、彩色 Gain Map
│       └── gainmap.rs / rotation.rs / update.rs / storage.rs
├── ui/                             # lumen-frame-ui
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── theme.rs                # GPUI 主题注册
│       ├── window_effects.rs       # 原生窗口背景与主题遮罩
│       ├── app.rs                  # 组合核心会话、页面状态与窗口资源
│       ├── behavior/               # 桌面任务调度与结果同步
│       ├── component/              # 控件与领域编辑器
│       ├── page/                   # 完整页面
│       ├── image.rs                # RGBA → GPUI BGRA 适配
│       └── image/layers.rs         # 按核心库像素身份复用 GPUI 纹理
└── mcp/                        # lumen-frame-mcp：工具描述、结果转换与传输
```

`assets/` 与 `test_images/` 仍在仓库根目录。版本和依赖版本集中在根 `Cargo.toml`；
发布脚本和构建图标的 `build.rs` 属于根应用包。根 `tests/` 对真实前后端 crate 做应用集成验证。

## 状态归属

| 状态 | 所有者 |
| --- | --- |
| 照片队列、选择、稳定身份 | `PhotoWorkspace` |
| 当前照片的水印参数与文字组真值 | 后端 `Session`；控件仅保留输入草稿 |
| 未选中照片的配置快照、全局新照片默认配置 | 后端 `Session`，快照按 `PhotoId` 索引 |
| 缩略图缓存 | `AppView`，按 `PhotoId` 索引 |
| 预览节奏、过期任务和 GPU 纹理 | 前端 `WatermarkPreview`；RGBA 图层缓存由后端持有 |
| 小工具页的 Gain Map / Motion Photo 会话 | `OtherToolsState` |
| 应用更新 UI 生命周期 | `UpdateState` + `crates/ui/src/behavior/update.rs` |
| 可持久化的应用偏好 | `persistence::settings::AppSettings` |

控件实体只保留输入、焦点、下拉开合等交互状态。业务值仍由上述所有者控制，避免第二份真值。

预览框直接叠加背景、透明阴影、照片外边框、圆角照片及各个文字组的缓存纹理，不在每次修改时先合成
整张位图。`Photo::prepare_watermark_layers` 是预览和 `Photo::compose_watermark` 共用的
排版管线，图层的渲染仍使用 `crates/core/src/render/` 的同一实现。导出继续完整合成并重建 Ultra HDR gain map。

后端缓存仅保留当前照片的已求值 `Arc<RasterImage>`，不跨线程传递 libvips 句柄。前端按像素对象身份复用 `Arc<RenderImage>`，保持未变图层的纹理身份。
水印文字通过 `crates/core/src/render/fonts.rs` 复用 GPUI 的平台字体后端。应用在主线程创建平台后，
把同一个 `PlatformTextSystem` 交给 `render::init_font_backend`；后台预览和导出只持有
可共享的字体接口，不创建窗口或平台对象。独立调用照片渲染的宿主也须先在主线程初始化。
字体选择与回退由 GPUI 处理，输出使用灰度抗锯齿覆盖率或彩色字形的 BGRA 像素，合成到
透明 RGBA 文字组图层；Logo、富文本装饰、文字组边框仍由图像渲染层组合。
照片路径、文件长度/修改时间或预览分辨率变化时清空所有层；背景按画布尺寸与背景参数失效，
照片按圆角失效，外边框按圆角、宽度和颜色失效，阴影按圆角、边框宽度、大小和浓度失效，各文字组按自身输入、EXIF 与默认字体/文字
配色环境独立失效。输出目录和 JPEG 质量不影响预览；最终旋转只更新图层方向和坐标。
文字大小或放置边改变了画布尺寸时，cover 背景需要重新生成，照片和阴影仍可复用。
未变图层保留纹理 ID，避免重复求值和上传。清空队列释放缓存，旧任务仍由请求版本拒绝。
`AppView::preview_image` 的完整位图只在显式读取时延迟压平，正常窗口绘制不走这条路径。

文字组的可见范围在该层生成时计算，只取非透明像素的范围；每次排版时裁到画布并随最终
输出旋转。预览请求同时冻结文字组的稳定 ID，返回的组下标只用于该快照内的对应。
界面在每帧布局后使用 GPUI 的 `ObjectFit::Contain` 映射区域，以标准按钮打开已有的文字组
编辑窗口；新请求一登记便停用旧图区域，点击事件也校验请求版本，防止切换照片或删除组后误编辑。

水印配置随照片独立保存：切换前将当前参数和文字组存为快照，切换后移出目标快照并恢复控件。
批量导出在启动时逐张冻结配置。预设默认应用「全部照片」，载入预设会覆盖队列并更新
后续导入的默认配置；也可以切换为「当前照片」。之后的手动画面调整仍只影响当前照片。
导出旋转方向由 `Session::params().rotation` 统一持有，预览与每张导出照片都使用当前角度，
不随照片切换、载入预设或恢复画面默认值改变；启动导出时与其他参数一起冻结。
输出目录和默认字体始终使用本机设置，不随照片切换。移除或清空队列时一并清理对应快照。
JPEG 质量是全部照片共用的本机导出偏好，调整后立即保存到 `settings.toml`，重启时恢复。
它不写入预设，也不随切换照片、载入预设或恢复画面默认值改变；每次导出逐张冻结当前质量。

`WatermarkParams::custom_text` 是当前照片的内容，随照片配置快照保存并在启动导出时冻结；
它不写入预设或应用设置。右侧输入框位于画布与边框上方，无选中照片时禁用。
载入当前/全部照片预设只替换样式和模板，保留各照片的自定义内容；新照片从空文本开始。
`{自定义文本}` 按原文插入，不二次解析其中的占位符，也不依赖 EXIF。
分层缓存只有引用该字段的文字组把它纳入失效键，其他文字与背景图层继续复用。

背景支持照片模糊、纯色与线性渐变。旧预设的 `solid_background` 继续兼容，
新增 `gradient_background` 启用渐变，`gradient` 保存经过校验的
`linear-gradient(角度deg,#颜色,...)` 字符串（至少一个 RGB 色标，单色时输出纯色）。界面通过角度数值、
九宫格中的八方向快捷按钮和横向色块修改，不直接暴露这段序列化字符串；角度不显示滑块。
色块悬停或点击可编辑、删除，拖动交换位置时颜色及其来源一起移动，颜色控件使用稳定身份，
删除其它色标或切换照片后，旧输入事件不能落到同一位置的新色标。
渐变角度沿用 CSS：0° 向上、90° 向右；RGB 通道线性插值，预览和导出共享
`crates/core/src/render/gradient.rs` 的 libvips 投影与 16 位查找表，渐变参数纳入背景图层缓存键。
自动文字颜色以渐变中点为参考，深浅跨度大的背景可手动指定文字颜色。

三种背景均可开启颗粒噪声：`background_grain` 默认关闭，`grain_strength` 保存 0–1 的强度。
`render/grain.rs` 使用固定种子的单通道高斯噪声，按照片高度缩放后叠加到背景 RGB，保留 alpha；
在照片、阴影与文字叠加前完成，预览和导出共用这一步。开关和强度只使背景图层缓存失效。

`features::photo_palette::extract_with_theme` 是可复用的取色入口：libvips 将原图按 EXIF 摆正、
缩小至长边 192 像素并转换为 sRGB，再由 `auto-palette` 提取代表色和面积占比。
支持默认均衡取色及 Basic、Colorful、Vivid、Muted、Light、Dark 全部 Theme；没有符合某个
Theme 的候选色时回退到原图代表色。纯 Rust 聚类结果按路径、文件大小与修改时间缓存，
至多保留 64 份；不同 Theme 共用聚类，界面结果按 `(PhotoId, Theme)` 独立保存。
移除照片时清理界面缓存，迟到任务不能写回已删除的照片，也不能覆盖当前照片的另一种 Theme。

色卡勾选即时同步到渐变，不另设添加步骤；首次勾选替换手动颜色，后续勾选加入、取消勾选移除。
全部添加选中整组并替换现有色标，重复点击不产生重复；最后一个勾选取消后保留为手动单色。
勾选状态由渐变来源推导，编辑、删除色标时同步取消对应勾选；`gradient_palette` 保存 Theme 和各色标的来源序号，手动色标用
`-1` 标记。切换照片、复制设置、应用预设和导出时按目标照片自己的色卡求值；代表色数量
不足时使用最后一个可用色。`photo_palette::resolve` 在预览和导出的后台入口共用，避免只更新
界面或把上一张照片的 RGB 固定复制到下一张。来源规则随预设保存，色卡缓存不写入预设，
取色不修改原图或 gain map。

纯色背景的 `background_palette_rank` 保存所选色卡序号，Theme 与照片边框共用
`gradient_palette.theme`；手动改色会清除此来源。预览、导出和颜色控件分别按目标照片解析，
不能把展示用 RGB 当成固定颜色规则。旧预设未保存来源时继续使用固定颜色。
保存非渐变预设时省略 `gradient_background`、`gradient` 与渐变来源 `slots`，仍保留取色 Theme。

`WatermarkParams::palette_card` 是独立的照片色卡水印，默认关闭。在参数面板点击「添加色卡」后，
通过独立窗口设置位置、对齐、方向、方形/圆形/圆角矩形、间距，以及色号和占比文字。
它与背景使用相同的取色缓存，但独立保存 Theme 和选中颜色的序号及顺序；应用到其它照片和导出时
重新解析目标照片的 RGB 与面积占比，派生颜色不写入预设。按占比模式将所选颜色归一化分配色条
长度，圆形使用等尺寸色块；色号图例等距排列，避免小面积色段的标签相互遮挡。
`crates/core/src/render/palette_card.rs` 先绘制 SVG 色块，色号通过同一 GPUI 字体后端叠加，再交给共享图层排版。
色卡和文字组共用 `crates/core/src/photo/layers.rs` 的排版描述与相邻树，相同位置共享一条区域。
色卡可通过「紧贴文字组」选择目标和上下左右方向，整体位置、对齐跟随目标组，组间距按照片高度计算。
菜单操作捕获稳定组 ID，在应用时映射到当前快照序号；删除文字组会同步修正序号或解除依附。
较大的相邻组合会扩展画布以保留首尾内容。分层预览单独缓存色卡，位置、依附和留白只更新坐标；
隐藏色卡时恢复原有画布计算。最终合成与 gain map 重建继续共用原路径。
色卡的可见点击区域由该图层的非透明像素生成，裁到画布后随输出旋转，并与文字组共用预览
中的标准按钮、contain 映射和请求版本保护。点击或键盘激活打开同一个色卡编辑窗口；新预览
请求登记时立即清除旧区域，重叠时优先命中最后绘制的色卡。

参数面板标题右侧可将当前样式、文字组和色卡来源规则应用到全部照片及后续导入，保留
每张照片自己的自定义文本和 EXIF；主动清空的文字组也按原样复制。已开始的导出使用冻结快照。

自定义 Logo 素材库由 `persistence::logos` 保存到系统数据目录下的 `lumen-frame/logos`，
macOS 对应 Application Support。文件使用稳定编号「自定义logo1」等，`logos.toml` 保存名称与
下一个编号，删除后不复用。默认字段为 `{自定义logo1}`；导入时填写名称则使用 `{名称logo}`，
旧编号作为兼容别名保留。预设只保存模板字段。黄色 Nikon SVG 嵌入为 `{尼康logo}` 内置示例，
不复制到用户目录，也不替换随 EXIF 明暗切换的品牌 Logo。`WatermarkParams::custom_logos` 是本机素材字节的不可变
快照，不序列化到预设，恢复照片和启动导出时使用当前素材库。预览和导出共用模板排版，
保留 PNG/SVG 的透明度与颜色；每行的 `TextParams::logo_size` 按照片高度独立设置
自定义 Logo 的可见高度，旧预设未设置时沿用字号比例。渲染时裁掉素材外围的透明留白，
SVG 按可见内容的最终尺寸栅格化；包含自定义 Logo 的行只以可见内容计高，行距作用于
行间，不给组的首尾额外增加字体行框空白。素材窗口的缩略图在后台生成。入口位于「文字与 Logo 水印」标题右侧，
独立窗口共享 AppView 状态，重复打开会激活已有窗口。

文字行的 `TextParams::spans` 保存模板 UTF-8 字节范围及局部字体、颜色、粗体、斜体和下划线覆盖，
未覆盖的属性继承整行设置。Textarea 内以界面默认字体显示模板；框内工具栏立即作用于当前选区，
选区为空时修改整行默认值及已有文字。最终字体、颜色和装饰在下方解析结果与照片预览中显示。
识别到的 `{标签}` 用 GPUI Kit 自带的 `InputContent` 和 token 编辑接口管理，保留底层字段文本。
字段菜单在当前光标位置插入；点击标签时用原生文本替换接口展开 `{字段名}`，离开后再识别为 token。
字段编辑、复制、选择与撤销使用上游输入组件，不引入依赖源码副本或本地补丁。
框内底部工具栏通过 Lucide 图标提供字体、颜色、粗斜体、下划线、插入字段与清除格式。
颜色通过 Popover 编辑，字体使用可搜索 Combobox；选区在弹层打开时保留。
控件通知匹配实际持有输入焦点的窗口同步选区，防止独立编辑窗口中的选区同步覆盖正在弹层里修改的颜色。
解析结果与照片渲染共用样式映射，按实际字段内容显示混排和下划线。
文本插入、替换、删除时按 Unicode 字符边界移动样式范围；新建文字行不继承上一行的局部范围。

模板字段作为完整对象设置样式，解析时把样式传给字段的实际内容，不能把模板的范围直接
套到字数不同的照片输出中。`{自定义文本}` 仍只展开一次；品牌 Logo 与自定义 Logo 作为
行内图像与 GPUI 排版结果按同一基线组合。连续文本用范围字体参与 GPUI 整行排版，按字形
的 UTF-8 索引应用颜色和装饰，保留字距、组对齐及竖排旋转。预览缓存按完整文字组失效，预设、照片切换与导出快照
均保留局部样式；新增格式不需要第二条预览/导出渲染管线。

文字组的相邻关系在编辑期间指向稳定组 ID，生成预设/渲染快照时投影成同一快照中的组序号。
恢复快照时重新映射到编辑器 ID，删除目标组会解除相邻关系，UI 禁止形成循环。共享排版层
按相邻组整体的包围盒计算边框和对齐，组间距为照片高度的比例；每个组仍保留独立纹理和
点击编辑区域。改变相邻位置只改变合成坐标，复用原有文字与背景缓存。

「导出当前」与「导出全部」共用 `crates/ui/src/behavior/export.rs` 的串行任务；当前照片按稳定
`PhotoId` 筛选，队列和输出目录在启动时冻结。清空队列不解除运行中的导出锁；成功导出
至少一张后，通过 GPUI 的系统打开接口显示本次输出目录（macOS 访达、Windows 文件资源管理器）。

预设面板的内置分组折叠状态与用户预设顺序属于应用偏好，保存到 `settings.toml`。
顺序以预设名为身份，忽略已移除的项，新预设追加到末尾；内置预设保持固定顺序。

文件夹扫描由后端 `import.rs` 实现，前端 `crates/ui/src/behavior/import.rs` 调度后台任务，选择和拖放共用 `add_photos` 入口。
扫描按路径排序、过滤图片格式，并用规范化路径去重和阻止符号链接循环；扫描结果复用
原有队列、缩略图与 EXIF 加载流程。清空队列会使之前的扫描结果失效。

## 新代码规则

1. 可序列化的水印输入放在 `crates/core/src/watermark/mod.rs`，渲染方法放在 `crates/core/src/render/`；持久化不得依赖渲染实现。
2. libvips 初始化和基础加载只经 `media::ensure_vips` / `media::load_base_image`。
3. EXIF 读取只经 `media::ExifInfo`；照片合成不自行解析文件容器。
4. 新的完整工作流放入 `features/`，并通过少量输入/输出类型暴露能力；不要塞回泛化的
   `process`、`helper` 或 `utils` 模块。
5. 设置与预设分别通过 `persistence::settings` 和 `persistence::presets`；UI 不拼配置路径。
6. `AppView` 只组合 feature 状态和窗口级资源。新增独立页面状态时优先创建专有状态类型，
   不继续增加一组无关字段。
7. 后台结果必须携带稳定身份或请求版本，旧任务不得覆盖新选择。
8. 后端不得依赖前端 crate；新业务操作同时提供后端入口。前端只添加控件、系统交互和异步调度。
9. 可外部调用的操作加入 `Command`，复用 `Session` 与现有媒体用例；不新增一份仅供 MCP 使用的状态。

## 测试边界

- `watermark`、`workspace`、时间/范围计算：纯单元测试。
- `media`、`render`、`gainmap`：真实夹具集成测试，并保留并发首次初始化测试。
- `persistence`：临时目录往返与损坏输入测试。
- UI 状态与交互：GPUI 测试上下文；图像视觉事实由预览集成测试覆盖。

## 独立构建与调用

```bash
cargo run
cargo check --workspace --all-targets
cargo check -p lumen-frame-core --lib
cargo test -p lumen-frame-core --test commands
cargo test -p lumen-frame-ui --lib
cargo run -p lumen-frame-core --example commands -- /tmp/lumen-demo
```

后端命令、宿主调度与未来 MCP 接入约定见 [core.md](core.md)。

MCP 对外只提供 stdio，默认桥接已打开的桌面工作区。`crates/mcp/src/ipc.rs` 负责本地
Unix Socket / Windows 命名管道、令牌校验和连接生命周期；`core::bridge` 只提供有界命令队列。
`ui::behavior::commands` 在 GPUI 主线程处理状态操作，在后台运行冻结快照，提交前检查版本，
并同步控件与预览。桥接进程不持有独立业务状态。`--mcp-headless` 才由工作线程持有独立 Session。
主窗口关闭时停止 IPC 与接收任务；桌面重启后桥接进程可重新连接。具体配置见 [mcp.md](mcp.md)。

文字组的 `TextGroup::background` 保存可选的 RGBA 背景，旧预设默认为关闭。
背景独立于描边开关，并与组边框共用圆角、内边距和最小尺寸；透明度为 0 时仍保留外框布局。
自动文字与品牌 Logo 颜色参考背景按 Alpha 叠加后的颜色；照片内文字在定位后采样实际覆盖区域，
再叠加组背景计算对比度。手动行颜色及富文本局部颜色优先，派生自动颜色不写回配置。
