(function () {
  "use strict";
  if (window.__rustwright) {
    return;
  }

  function isVisible(el) {
    if (!el || !el.getBoundingClientRect) {
      return false;
    }
    var style = window.getComputedStyle(el);
    if (!style) {
      return false;
    }
    if (
      style.visibility === "hidden" ||
      style.visibility === "collapse" ||
      style.display === "none"
    ) {
      return false;
    }
    var rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      return false;
    }
    return true;
  }

  function isEnabled(el) {
    if (!el) {
      return false;
    }
    return !el.disabled && el.getAttribute("aria-disabled") !== "true";
  }

  function implicitRole(el) {
    var explicit = el.getAttribute("role");
    if (explicit) {
      return explicit.trim().toLowerCase();
    }
    var tag = el.tagName ? el.tagName.toLowerCase() : "";
    var type = (el.getAttribute("type") || "").toLowerCase();
    if (tag === "a" || tag === "area") {
      return el.hasAttribute("href") ? "link" : "generic";
    }
    if (tag === "button") {
      return "button";
    }
    if (tag === "input") {
      if (type === "checkbox") return "checkbox";
      if (type === "radio") return "radio";
      if (type === "range") return "slider";
      if (type === "number") return "spinbutton";
      if (
        type === "submit" ||
        type === "image" ||
        type === "reset" ||
        type === "button"
      ) {
        return "button";
      }
      if (type === "search") return "searchbox";
      return "textbox";
    }
    if (tag === "textarea") return "textbox";
    if (tag === "select") {
      if (el.multiple || (el.size && el.size > 1)) return "listbox";
      return "combobox";
    }
    if (/^h[1-6]$/.test(tag)) return "heading";
    if (tag === "img") {
      return el.getAttribute("alt") === "" ? "presentation" : "img";
    }
    if (tag === "ul" || tag === "ol") return "list";
    if (tag === "li") return "listitem";
    if (tag === "nav") return "navigation";
    if (tag === "main") return "main";
    if (tag === "header") return "banner";
    if (tag === "footer") return "contentinfo";
    if (tag === "aside") return "complementary";
    if (tag === "form") return "form";
    if (tag === "table") return "table";
    if (tag === "tr") return "row";
    if (tag === "td") return "cell";
    if (tag === "th") return "columnheader";
    if (tag === "dialog") return "dialog";
    if (tag === "section") return "region";
    if (tag === "article") return "article";
    if (tag === "progress") return "progressbar";
    if (tag === "option") return "option";
    return "generic";
  }

  function accessibleName(el) {
    var aria = el.getAttribute("aria-label");
    if (aria && aria.trim()) {
      return aria.trim();
    }
    var labelledby = el.getAttribute("aria-labelledby");
    if (labelledby) {
      var parts = labelledby.split(/\s+/).map(function (id) {
        var node = document.getElementById(id);
        return node ? node.innerText || node.textContent || "" : "";
      });
      var joined = parts.join(" ").trim();
      if (joined) {
        return joined;
      }
    }
    if (el.tagName === "IMG") {
      var alt = el.getAttribute("alt");
      if (alt && alt.trim()) {
        return alt.trim();
      }
    }
    if (el.labels && el.labels.length) {
      var labelText = Array.prototype.map
        .call(el.labels, function (label) {
          return label.innerText || label.textContent || "";
        })
        .join(" ")
        .trim();
      if (labelText) {
        return labelText;
      }
    }
    var placeholder = el.getAttribute("placeholder");
    if (placeholder && placeholder.trim()) {
      return placeholder.trim();
    }
    var title = el.getAttribute("title");
    if (title && title.trim()) {
      return title.trim();
    }
    return (el.innerText || el.textContent || "").trim();
  }

  function matchesText(actual, expected, exact) {
    if (exact) {
      return actual === expected;
    }
    return actual.toLowerCase().indexOf(expected.toLowerCase()) !== -1;
  }

  function queryAll(spec) {
    var result = [];
    if (!spec) {
      return result;
    }
    if (spec.kind === "css") {
      try {
        return Array.prototype.slice.call(document.querySelectorAll(spec.value));
      } catch (error) {
        return result;
      }
    }
    if (spec.kind === "placeholder") {
      var placeholders = document.querySelectorAll("[placeholder]");
      for (var p = 0; p < placeholders.length; p += 1) {
        if (
          matchesText(
            placeholders[p].getAttribute("placeholder") || "",
            spec.value,
            spec.exact
          )
        ) {
          result.push(placeholders[p]);
        }
      }
      return result;
    }
    if (spec.kind === "alt") {
      var images = document.querySelectorAll("[alt]");
      for (var a = 0; a < images.length; a += 1) {
        if (matchesText(images[a].getAttribute("alt") || "", spec.value, spec.exact)) {
          result.push(images[a]);
        }
      }
      return result;
    }
    if (spec.kind === "testid") {
      var attributes = ["data-testid", "data-test-id", "data-test"];
      var selector = attributes
        .map(function (attribute) {
          return "[" + attribute + "=" + JSON.stringify(spec.value) + "]";
        })
        .join(",");
      try {
        return Array.prototype.slice.call(document.querySelectorAll(selector));
      } catch (error) {
        return result;
      }
    }
    if (spec.kind === "label") {
      var labels = document.querySelectorAll("label");
      for (var l = 0; l < labels.length; l += 1) {
        var label = labels[l];
        var labelText = (label.innerText || label.textContent || "").trim();
        if (!matchesText(labelText, spec.value, spec.exact)) {
          continue;
        }
        var control = label.control;
        if (!control && label.htmlFor) {
          control = document.getElementById(label.htmlFor);
        }
        if (control && result.indexOf(control) === -1) {
          result.push(control);
        }
      }
      var aria = document.querySelectorAll("[aria-label]");
      for (var r = 0; r < aria.length; r += 1) {
        if (
          matchesText(
            aria[r].getAttribute("aria-label") || "",
            spec.value,
            spec.exact
          ) &&
          result.indexOf(aria[r]) === -1
        ) {
          result.push(aria[r]);
        }
      }
      return result;
    }
    var all = document.querySelectorAll("body *");
    for (var i = 0; i < all.length; i += 1) {
      var el = all[i];
      if (spec.kind === "text") {
        var text = (el.innerText || el.textContent || "").trim();
        if (matchesText(text, spec.value, spec.exact)) {
          result.push(el);
        }
      } else if (spec.kind === "role") {
        if (implicitRole(el) !== spec.role) {
          continue;
        }
        if (
          spec.name == null ||
          matchesText(accessibleName(el), spec.name, spec.exact)
        ) {
          result.push(el);
        }
      }
    }
    return result;
  }

  function resolve(spec) {
    if (!spec) {
      return null;
    }
    var list = queryAll(spec);
    if (spec.nth != null) {
      var index = spec.nth;
      if (index < 0) {
        index = list.length + index;
      }
      return index >= 0 && index < list.length ? list[index] : null;
    }
    if (list.length === 0) {
      return null;
    }
    if (spec.kind === "text") {
      return list[list.length - 1];
    }
    return list[0];
  }

  function stateSatisfied(spec, state) {
    var el = resolve(spec);
    if (state === "attached") {
      return !!el;
    }
    if (state === "detached") {
      return !el;
    }
    if (state === "visible") {
      return !!el && isVisible(el);
    }
    if (state === "hidden") {
      return !el || !isVisible(el);
    }
    return false;
  }

  function waitFor(spec, state, timeout) {
    return new Promise(function (resolvePromise) {
      var start = Date.now();
      var settled = false;
      var observer = new MutationObserver(function () {
        if (settled) {
          return;
        }
        if (stateSatisfied(spec, state)) {
          settle(true);
        }
      });
      // A MutationObserver reacts to DOM changes, while the interval is a
      // safety net for changes that do not mutate the tree (for example a
      // computed-style/visibility change) and for background tabs where
      // requestAnimationFrame is throttled.
      var timer = setInterval(function () {
        if (settled) {
          return;
        }
        if (stateSatisfied(spec, state)) {
          settle(true);
          return;
        }
        if (Date.now() - start >= timeout) {
          settle(false);
        }
      }, 50);

      function settle(result) {
        if (settled) {
          return;
        }
        settled = true;
        observer.disconnect();
        clearInterval(timer);
        resolvePromise(result);
      }

      if (stateSatisfied(spec, state)) {
        settle(true);
        return;
      }
      observer.observe(document.documentElement || document, {
        childList: true,
        subtree: true,
        attributes: true,
        characterData: true,
      });
    });
  }

  window.__rustwright = {
    resolve: resolve,
    queryAll: queryAll,
    isVisible: function (spec) {
      return isVisible(resolve(spec));
    },
    isEnabled: function (spec) {
      return isEnabled(resolve(spec));
    },
    count: function (spec) {
      if (spec && spec.nth != null) {
        return resolve(spec) ? 1 : 0;
      }
      return queryAll(spec).length;
    },
    waitFor: waitFor,
  };
})();
