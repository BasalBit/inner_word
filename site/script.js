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

  // After a download starts, show the first-launch steps: an unsigned app is blocked
  // the first time, and the reader should meet the steps before macOS's warning.
  var install = document.getElementById("install");
  var status = document.querySelector(".dl__status");
  document.querySelectorAll("[data-dmg]").forEach(function (a) {
    a.addEventListener("click", function () {
      if (a.getAttribute("href").charAt(0) === "#" || !install) return;
      setTimeout(function () {
        if (status) status.textContent = "Your download has started. macOS will ask you to approve Inner Word once: here is how.";
        if (history.replaceState) history.replaceState(null, "", "#install");
        install.scrollIntoView({ block: "start" });
        install.focus({ preventScroll: true });
      }, 60);
    });
  });

  // On a phone or a tablet a disk image is of no use: the hero's button leads to the
  // download section, which says to open the page on a Mac.
  var heroGet = document.querySelector("[data-hero-get]");
  if (heroGet) {
    var label = heroGet.querySelector("[data-label]");
    var dmgHref = heroGet.href;
    var phone = window.matchMedia("(max-width: 600px), (pointer: coarse)");
    var place = function () {
      heroGet.href = phone.matches ? "#download" : dmgHref;
      if (label) label.textContent = phone.matches ? "Get it for your Mac" : "Download for macOS";
    };
    place();
    if (phone.addEventListener) phone.addEventListener("change", place);
  }

  // Copy buttons: a command (data-copy names its element) or this page's address.
  var copyText = function (button, text, idle) {
    var done = function (label) {
      button.textContent = label;
      setTimeout(function () { button.textContent = idle; }, 1600);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(function () { done("Copied"); }, function () { done("Select it"); });
    } else {
      done("Select it");
    }
  };
  document.querySelectorAll("[data-copy]").forEach(function (button) {
    button.addEventListener("click", function () {
      copyText(button, document.getElementById(button.getAttribute("data-copy")).textContent, "Copy");
    });
  });
  document.querySelectorAll("[data-copy-url]").forEach(function (button) {
    button.addEventListener("click", function () {
      copyText(button, location.href.split("#")[0], "Copy link");
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
