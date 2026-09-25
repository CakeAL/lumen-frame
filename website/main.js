const releaseUrl = "https://github.com/CakeAL/lumen-frame/releases/latest";
const patterns = {
  "mac-arm": /^Lumen-Frame-.+-macOS-arm64\.dmg$/i,
  "mac-intel": /^Lumen-Frame-.+-macOS-x86_64\.dmg$/i,
  windows: /^Lumen-Frame-.+-Windows-x86_64\.zip$/i,
};

const menuToggle = document.querySelector(".menu-toggle");
const navLinks = document.querySelector(".nav-links");

menuToggle.addEventListener("click", () => {
  const expanded = menuToggle.getAttribute("aria-expanded") === "true";
  menuToggle.setAttribute("aria-expanded", String(!expanded));
  menuToggle.setAttribute("aria-label", expanded ? "打开导航菜单" : "关闭导航菜单");
  navLinks.classList.toggle("is-open", !expanded);
});

navLinks.addEventListener("click", (event) => {
  if (event.target.closest("a")) {
    menuToggle.setAttribute("aria-expanded", "false");
    menuToggle.setAttribute("aria-label", "打开导航菜单");
    navLinks.classList.remove("is-open");
  }
});

async function loadLatestRelease() {
  try {
    const response = await fetch("https://api.github.com/repos/CakeAL/lumen-frame/releases/latest", {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`GitHub API: ${response.status}`);

    const release = await response.json();
    if (!Array.isArray(release.assets)) throw new Error("缺少 Release 文件列表");

    for (const link of document.querySelectorAll(".asset-link")) {
      const asset = release.assets.find((item) => patterns[link.dataset.asset].test(item.name));
      if (!asset?.browser_download_url) continue;
      link.href = asset.browser_download_url;
      link.innerHTML = `下载${link.dataset.asset === "windows" ? " ZIP" : " DMG"} <span aria-hidden="true">↗</span>`;
      link.setAttribute("aria-label", `${link.closest(".download-card").querySelector("h3").textContent}，下载最新版本 ${release.tag_name}`);
    }

    if (release.tag_name) {
      document.querySelector("#release-version").textContent = `当前版本 ${release.tag_name}`;
    }
  } catch {
    // 离线或 GitHub API 暂不可用时，各按钮仍可打开最新 Release 页面。
    for (const link of document.querySelectorAll(".asset-link")) link.href = releaseUrl;
  }
}

loadLatestRelease();
