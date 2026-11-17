// Inner Word landing page. No dependencies, no requests of its own.
// The release is named in ONE place: the data-version attribute on <html> (with the
// repository and the sizes of the two disk images beside it). Everything below is built
// from it. Without this script the download links lead to the releases page.
(function () {
  "use strict";
  var root = document.documentElement;
  root.classList.add("js");

  var version = root.getAttribute("data-version");
  var repo = root.getAttribute("data-repo");

  // Download links: <a data-dmg="universal|arm64">
  document.querySelectorAll("[data-dmg]").forEach(function (a) {
    var kind = a.getAttribute("data-dmg");
    a.href = repo + "/releases/download/v" + version + "/Inner-Word-" + version + "-" + kind + ".dmg";
  });
  document.querySelectorAll("[data-v]").forEach(function (el) { el.textContent = version; });
  document.querySelectorAll("[data-size]").forEach(function (el) {
    el.textContent = root.getAttribute("data-size-" + el.getAttribute("data-size")) || "";
  });

  // Paper / Night switch for the hero screenshot. It starts on the scheme the page is in.
  var picture = document.querySelector("[data-theme-picture]");
  var buttons = document.querySelectorAll("[data-show-theme]");
  if (picture && buttons.length) {
    // The dark sources carry their media without the colour scheme in data-media
    // (the hero has a phone crop and a window shot, each in both themes).
    var darkSources = picture.querySelectorAll("source[data-media]");
    var dark = window.matchMedia("(prefers-color-scheme: dark)");
    var chosen = null;
    var show = function (theme) {
      darkSources.forEach(function (source) {
        source.media = theme === "dark" ? source.getAttribute("data-media") : "not all";
      });
      buttons.forEach(function (b) {
        b.setAttribute("aria-pressed", String(b.getAttribute("data-show-theme") === theme));
      });
    };
    show(dark.matches ? "dark" : "light");
    buttons.forEach(function (b) {
      b.addEventListener("click", function () { chosen = b.getAttribute("data-show-theme"); show(chosen); });
    });
    var follow = function () { if (!chosen) show(dark.matches ? "dark" : "light"); };
    if (dark.addEventListener) dark.addEventListener("change", follow);
  }

  // Copy buttons for commands.
  document.querySelectorAll("[data-copy]").forEach(function (button) {
    button.addEventListener("click", function () {
      var text = document.getElementById(button.getAttribute("data-copy")).textContent;
      var done = function (label) {
        button.textContent = label;
        setTimeout(function () { button.textContent = "Copy"; }, 1600);
      };
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(function () { done("Copied"); }, function () { done("Select it"); });
      } else {
        done("Select it");
      }
    });
  });

  // Screenshots rise in as they come into view.
  var reveals = document.querySelectorAll(".reveal");
  if ("IntersectionObserver" in window) {
    var io = new IntersectionObserver(function (entries) {
      entries.forEach(function (e) {
        if (e.isIntersecting) { e.target.classList.add("is-in"); io.unobserve(e.target); }
      });
    }, { rootMargin: "0px 0px -8% 0px", threshold: 0.05 });
    reveals.forEach(function (el) { io.observe(el); });
  } else {
    reveals.forEach(function (el) { el.classList.add("is-in"); });
  }
})();
