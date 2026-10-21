# 网站维护

网站使用原生 HTML、CSS 和 JavaScript，发布到 GitHub Pages。功能与接入说明以根目录
`README.md` 为准；界面和预设效果直接复用 `pics/`，无需在网站目录重复保存。

## 本地预览

在仓库根目录运行：

```bash
python3 script/build-website.py
python3 -m http.server 4173 --bind 127.0.0.1 --directory target/website
```

打开 `http://127.0.0.1:4173`。修改页面或图片后，重新运行构建命令并刷新浏览器。
构建会检查页面引用的本地素材是否存在。JavaScript 语法可用 `node --check website/main.js` 检查。

预设效果默认折叠，展开后支持左右按钮、方向键、名称跳转及大图查看。MCP 配置支持切换
mcpServers JSON、Codex、Deepseek Harness 示例与复制（JSON 中包含 macOS 与 Windows 配置）；下载区通过 GitHub API 获取最新正式版，网络不可用时
保留 Release 页面入口。全部动效遵循系统的减少动态效果设置。

推送 `website/`、`pics/` 或构建脚本的修改到 `main` 时，GitHub Actions 会构建并部署到
`https://cakeal.github.io/lumen-frame/`。
