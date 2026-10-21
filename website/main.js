const releaseUrl = "https://github.com/CakeAL/lumen-frame/releases/latest";
const patterns = {
  "mac-arm": /^Lumen-Frame-.+-macOS-arm64\.dmg$/i,
  "mac-intel": /^Lumen-Frame-.+-macOS-x86_64\.dmg$/i,
  windows: /^Lumen-Frame-.+-Windows-x86_64\.zip$/i,
};
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
const menuToggle = document.querySelector(".menu-toggle");
const navLinks = document.querySelector(".nav-links");
function closeMenu() {
  menuToggle.setAttribute("aria-expanded", "false");
  menuToggle.setAttribute("aria-label", "打开导航菜单");
  navLinks.classList.remove("is-open");
}
menuToggle.addEventListener("click", () => {
  const expanded = menuToggle.getAttribute("aria-expanded") === "true";
  menuToggle.setAttribute("aria-expanded", String(!expanded));
  menuToggle.setAttribute("aria-label", expanded ? "打开导航菜单" : "关闭导航菜单");
  navLinks.classList.toggle("is-open", !expanded);
});
navLinks.addEventListener("click", event => { if (event.target.closest("a")) closeMenu(); });
document.addEventListener("keydown", event => {
  if (event.key === "Escape" && menuToggle.getAttribute("aria-expanded") === "true") {
    closeMenu();
    menuToggle.focus();
  }
});

const gallery = document.querySelector("#presets");
const track = document.querySelector("#preset-track");
const slides = [...track.querySelectorAll(".preset-slide")];
const selectors = [...document.querySelectorAll("[data-slide]")];
const prev = document.querySelector(".carousel-prev");
const next = document.querySelector(".carousel-next");
const count = document.querySelector("#carousel-count");
let currentSlide = 0;
function updateCarousel(index) {
  currentSlide = Math.max(0, Math.min(slides.length - 1, index));
  count.textContent = `${String(currentSlide + 1).padStart(2, "0")} / ${String(slides.length).padStart(2, "0")}`;
  selectors.forEach((button, ix) => {
    if (ix === currentSlide) button.setAttribute("aria-current", "true");
    else button.removeAttribute("aria-current");
  });
  // 非当前页不进入键盘焦点顺序，方向键仍可在轮播区域切换。
  slides.forEach((slide, ix) => {
    const active = ix === currentSlide;
    slide.querySelector("button").tabIndex = active ? 0 : -1;
    if (active) slide.removeAttribute("aria-hidden");
    else slide.setAttribute("aria-hidden", "true");
  });
  prev.disabled = currentSlide === 0;
  next.disabled = currentSlide === slides.length - 1;
}
function showSlide(index, behavior = reducedMotion.matches ? "instant" : "smooth") {
  const target = Math.max(0, Math.min(slides.length - 1, index));
  updateCarousel(target);
  track.scrollTo({ left: slides[target].offsetLeft - slides[0].offsetLeft, behavior });
}
prev.addEventListener("click", () => showSlide(currentSlide - 1));
next.addEventListener("click", () => showSlide(currentSlide + 1));
selectors.forEach((button, ix) => button.addEventListener("click", () => showSlide(ix)));
let scrollFrame = 0;
track.addEventListener("scroll", () => {
  if (scrollFrame) return;
  scrollFrame = requestAnimationFrame(() => {
    scrollFrame = 0;
    if (track.clientWidth) updateCarousel(Math.round(track.scrollLeft / track.clientWidth));
  });
}, { passive: true });
track.addEventListener("keydown", event => {
  if (["ArrowRight", "ArrowLeft", "Home", "End"].includes(event.key)) {
    event.preventDefault();
    const index = event.key === "Home" ? 0 : event.key === "End" ? slides.length - 1 : currentSlide + (event.key === "ArrowRight" ? 1 : -1);
    // 避免焦点停留在随后移出可视区域的放大按钮上。
    if (event.target !== track) track.focus({ preventScroll: true });
    showSlide(index);
  }
});
gallery.addEventListener("toggle", () => { if (gallery.open) showSlide(currentSlide, "instant"); });
let galleryWidth = 0;
new ResizeObserver(() => {
  if (track.clientWidth && track.clientWidth !== galleryWidth) {
    galleryWidth = track.clientWidth;
    showSlide(currentSlide, "instant");
  }
}).observe(track);
updateCarousel(0);

const configTabs = [...document.querySelectorAll('[role="tab"]')];
function selectConfig(tab) {
  configTabs.forEach(button => {
    const selected = button === tab;
    button.setAttribute("aria-selected", String(selected));
    button.tabIndex = selected ? 0 : -1;
    document.getElementById(button.getAttribute("aria-controls")).hidden = !selected;
  });
}
configTabs.forEach((tab, ix) => {
  tab.addEventListener("click", () => selectConfig(tab));
  tab.addEventListener("keydown", event => {
    if (!["ArrowRight", "ArrowLeft", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const target = event.key === "Home" ? 0 : event.key === "End" ? configTabs.length - 1 : (ix + (event.key === "ArrowRight" ? 1 : -1) + configTabs.length) % configTabs.length;
    selectConfig(configTabs[target]);
    configTabs[target].focus();
  });
});
for (const button of document.querySelectorAll("[data-copy]")) {
  let feedbackTimer;
  button.addEventListener("click", async () => {
    const code = document.getElementById(button.dataset.copy);
    const status = document.querySelector("#copy-status");
    clearTimeout(feedbackTimer);
    try {
      await navigator.clipboard.writeText(code.textContent);
      button.textContent = "已复制";
      status.textContent = "配置已复制到剪贴板";
    } catch {
      const range = document.createRange();
      range.selectNodeContents(code);
      const selection = window.getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      button.textContent = "请手动复制";
      status.textContent = "无法访问剪贴板，已选中配置，请按系统复制快捷键";
    }
    feedbackTimer = setTimeout(() => { button.textContent = "复制配置"; }, 2200);
  });
}

const viewer = document.querySelector("#image-viewer");
const viewerImage = document.querySelector("#viewer-image");
let viewerTrigger;
for (const button of document.querySelectorAll("[data-image]")) {
  button.addEventListener("click", () => {
    viewerTrigger = button;
    viewerImage.src = button.dataset.image;
    viewerImage.alt = button.dataset.caption;
    document.querySelector("#viewer-caption").textContent = button.dataset.caption;
    viewer.showModal();
    document.body.classList.add("viewer-open");
  });
}
document.querySelector(".viewer-close").addEventListener("click", () => viewer.close());
viewer.addEventListener("click", event => {
  const bounds = viewer.getBoundingClientRect();
  if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) viewer.close();
});
viewer.addEventListener("close", () => {
  document.body.classList.remove("viewer-open");
  viewerImage.removeAttribute("src");
  viewerTrigger?.focus({ preventScroll: true });
});

function recommend(platform, architecture) {
  const row = document.querySelector(`.download-row[data-platform="${platform}"]${architecture ? `[data-arch="${architecture}"]` : ""}`);
  if (!row) return;
  row.classList.add("is-recommended");
  row.querySelector(".recommended").hidden = false;
}

async function detectSystem() {
  const userAgent = navigator.userAgent;
  const platform = navigator.userAgentData?.platform || navigator.platform || "";
  const detected = document.querySelector("#detected-system");
  if (/Windows/i.test(platform) || /Windows/i.test(userAgent)) {
    detected.textContent = "检测到 Windows；已标出对应版本。";
    recommend("windows");
    return;
  }
  if (/Mac/i.test(platform) || /Macintosh/i.test(userAgent)) {
    let architecture = "";
    if (navigator.userAgentData?.getHighEntropyValues) {
      try {
        const hints = await navigator.userAgentData.getHighEntropyValues(["architecture"]);
        if (/arm/i.test(hints.architecture)) architecture = "arm";
        if (/x86/i.test(hints.architecture)) architecture = "intel";
      } catch { /* 浏览器未提供芯片信息。 */ }
    }
    detected.textContent = architecture ? "检测到 macOS；已标出对应芯片版本。" : "检测到 macOS；请选择 Apple Silicon 或 Intel 版本。";
    if (architecture) recommend("mac", architecture);
    return;
  }
  if (/Linux/i.test(platform) || /Linux/i.test(userAgent)) detected.textContent = "当前为 Linux；可从 GitHub 仓库获取源码自行构建。";
}
detectSystem();

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
      link.textContent = link.dataset.asset === "windows" ? "下载 ZIP ↗" : "下载 DMG ↗";
      link.setAttribute("aria-label", `${link.closest(".download-row").querySelector("h3").textContent}，下载最新版本 ${release.tag_name}`);
    }
    if (release.tag_name) document.querySelector("#release-version").textContent = `最新正式版：${release.tag_name}`;
  } catch {
    // 离线或 GitHub API 暂不可用时，链接仍可打开最新 Release 页面。
    for (const link of document.querySelectorAll(".asset-link")) link.href = releaseUrl;
  }
}
loadLatestRelease();
