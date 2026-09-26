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

const track = document.querySelector(".carousel-track");
const slides = [...track.querySelectorAll(".carousel-slide")];
const dots = [...document.querySelectorAll(".carousel-dots button")];
const prev = document.querySelector(".carousel-prev");
const next = document.querySelector(".carousel-next");
const count = document.querySelector("#carousel-count");
let currentSlide = 0;

function updateCarousel(index) {
  currentSlide = Math.max(0, Math.min(slides.length - 1, index));
  count.textContent = `${String(currentSlide + 1).padStart(2, "0")} / ${String(slides.length).padStart(2, "0")}`;
  dots.forEach((dot, dotIndex) => {
    if (dotIndex === currentSlide) dot.setAttribute("aria-current", "true");
    else dot.removeAttribute("aria-current");
  });
  prev.disabled = currentSlide === 0;
  next.disabled = currentSlide === slides.length - 1;
}
function showSlide(index) {
  const targetIndex = Math.max(0, Math.min(slides.length - 1, index));
  track.scrollTo({
    left: slides[targetIndex].offsetLeft - slides[0].offsetLeft,
    behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
  });
  updateCarousel(targetIndex);
}
prev.addEventListener("click", () => showSlide(currentSlide - 1));
next.addEventListener("click", () => showSlide(currentSlide + 1));
dots.forEach((dot, index) => dot.addEventListener("click", () => showSlide(index)));
track.addEventListener("scroll", () => {
  const index = Math.round(track.scrollLeft / track.clientWidth);
  updateCarousel(index);
}, { passive: true });
track.addEventListener("keydown", (event) => {
  if (event.key === "ArrowRight" || event.key === "ArrowLeft") {
    event.preventDefault();
    showSlide(currentSlide + (event.key === "ArrowRight" ? 1 : -1));
  }
});
updateCarousel(0);

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
