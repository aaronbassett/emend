// Preview bridge (US4 · research §C2/§C3). The WKWebView host calls
// window.__emendRender(html, css) after each (debounced) core render to inject the
// comrak HTML + syntect theme CSS, then render Mermaid diagrams and KaTeX math.
// Everything runs locally — the page CSP and the WKWebView host forbid any
// network access (SC-008). Loaded via <script src="bridge.js"> (CSP 'self').
"use strict";

(function () {
  function injectThemeCSS(css) {
    var el = document.getElementById("emend-syntect");
    if (!el) {
      el = document.createElement("style");
      el.id = "emend-syntect";
      document.head.appendChild(el);
    }
    el.textContent = css;
  }

  // comrak emits ```mermaid as <pre><code class="language-mermaid">SOURCE</code></pre>
  // (syntect leaves unknown languages as escaped plain text, so textContent is the
  // raw diagram source). Convert each to a <div class="mermaid"> and run Mermaid.
  // Returns the Mermaid promise so the print path can await async layout (§C4).
  function renderMermaid(root) {
    var blocks = root.querySelectorAll("pre > code.language-mermaid");
    blocks.forEach(function (code) {
      var div = document.createElement("div");
      div.className = "mermaid";
      div.textContent = code.textContent;
      var pre = code.parentNode;
      pre.parentNode.replaceChild(div, pre);
    });
    if (window.mermaid && root.querySelector(".mermaid")) {
      try {
        return window.mermaid.run({ nodes: root.querySelectorAll(".mermaid") });
      } catch (e) {
        /* malformed diagram — leave the source visible, never throw */
      }
    }
    return Promise.resolve();
  }

  // KaTeX 0.17 has no auto-render bundled, so walk text nodes (skipping code/pre)
  // and replace $$display$$ / $inline$ with rendered spans. Conservative: a stray
  // single $ (currency) without a matching close is left as text.
  function renderMath(root) {
    if (!window.katex) return;
    var walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode: function (node) {
        for (var p = node.parentNode; p && p !== root; p = p.parentNode) {
          var t = p.tagName;
          if (t === "CODE" || t === "PRE" || t === "SCRIPT" || t === "STYLE") {
            return NodeFilter.FILTER_REJECT;
          }
        }
        return node.nodeValue.indexOf("$") >= 0
          ? NodeFilter.FILTER_ACCEPT
          : NodeFilter.FILTER_REJECT;
      },
    });
    var targets = [];
    for (var n = walker.nextNode(); n; n = walker.nextNode()) targets.push(n);
    targets.forEach(function (node) {
      var text = node.nodeValue;
      var re = /\$\$([\s\S]+?)\$\$|\$([^$\n]+?)\$/g;
      var frag = document.createDocumentFragment();
      var last = 0;
      var m;
      while ((m = re.exec(text))) {
        if (m.index > last) {
          frag.appendChild(document.createTextNode(text.slice(last, m.index)));
        }
        var display = m[1] !== undefined;
        var span = document.createElement("span");
        try {
          window.katex.render(display ? m[1] : m[2], span, {
            displayMode: display,
            throwOnError: false,
          });
        } catch (e) {
          span.textContent = m[0];
        }
        frag.appendChild(span);
        last = re.lastIndex;
      }
      if (last > 0) {
        if (last < text.length) {
          frag.appendChild(document.createTextNode(text.slice(last)));
        }
        node.parentNode.replaceChild(frag, node);
      }
    });
  }

  // --- Scroll sync (US4 · research §C3) -----------------------------------
  // The host (ScrollSync.swift) drives the preview via window.__emendScrollToLine
  // and receives the top visible source line via the "emendScroll" message handler.
  // Both sides key on comrak's 1-based data-line anchors.

  var anchors = []; // sorted [{ line, top }] in document-scroll coordinates
  var applyingScroll = false; // ignore the scroll events our own scrollTo causes
  var scrollTimer = null;
  var resizeObserver = null;

  // Rebuild the anchor table from every [data-line] block. getBoundingClientRect
  // forces layout synchronously, so tops are accurate right after innerHTML.
  function buildAnchorTable() {
    anchors = [];
    // comrak's sourcepos lands data-line on inline nodes too (a <code>/<em>/<a>
    // shares its block's line), so keep only the FIRST element per line — in
    // document order that's the block, whose top is the line's true position.
    var seen = {};
    var els = document.querySelectorAll("[data-line]");
    for (var i = 0; i < els.length; i++) {
      var line = parseInt(els[i].getAttribute("data-line"), 10);
      if (!isNaN(line) && !seen[line]) {
        seen[line] = true;
        anchors.push({
          line: line,
          top: els[i].getBoundingClientRect().top + window.scrollY,
        });
      }
    }
    anchors.sort(function (a, b) {
      return a.line - b.line;
    });
  }

  // Interpolated document top for a source line (linear between bracketing anchors).
  function topForLine(line) {
    if (!anchors.length) return 0;
    if (line <= anchors[0].line) return anchors[0].top;
    for (var i = 0; i < anchors.length; i++) {
      if (anchors[i].line === line) return anchors[i].top;
      if (anchors[i].line > line) {
        var a = anchors[i - 1];
        var b = anchors[i];
        var t = (line - a.line) / (b.line - a.line);
        return a.top + t * (b.top - a.top);
      }
    }
    return anchors[anchors.length - 1].top;
  }

  // The source line at the current scroll top (last anchor at or above the fold).
  function topVisibleLine() {
    if (!anchors.length) return 1;
    var y = window.scrollY;
    var line = anchors[0].line;
    for (var i = 0; i < anchors.length; i++) {
      if (anchors[i].top <= y + 1) line = anchors[i].line;
      else break;
    }
    return line;
  }

  window.__emendScrollToLine = function (line) {
    if (!anchors.length) return;
    applyingScroll = true;
    window.scrollTo(0, Math.round(topForLine(line)));
    // Release on a later frame so the resulting scroll events are swallowed.
    window.requestAnimationFrame(function () {
      window.requestAnimationFrame(function () {
        applyingScroll = false;
      });
    });
  };

  function reportScroll() {
    scrollTimer = null;
    if (applyingScroll) return;
    var handlers = window.webkit && window.webkit.messageHandlers;
    if (handlers && handlers.emendScroll) {
      handlers.emendScroll.postMessage({ line: topVisibleLine() });
    }
  }

  // Throttle scroll reports to ~60 ms so a flick doesn't flood the host.
  window.addEventListener(
    "scroll",
    function () {
      if (applyingScroll || scrollTimer) return;
      scrollTimer = window.setTimeout(reportScroll, 60);
    },
    { passive: true }
  );

  window.__emendRender = function (html, css) {
    injectThemeCSS(css);
    var content = document.getElementById("emend-content");
    if (!content) return;
    content.innerHTML = html;
    renderMermaid(content);
    renderMath(content);
    buildAnchorTable();
    // Mermaid/KaTeX lay out asynchronously and shift block tops, so rebuild the
    // anchor table whenever the content box resizes (research §C3).
    if (!resizeObserver && window.ResizeObserver) {
      resizeObserver = new ResizeObserver(function () {
        buildAnchorTable();
      });
      resizeObserver.observe(content);
    }
    if (typeof window.__emendAfterRender === "function") {
      window.__emendAfterRender();
    }
  };

  // PDF export (US4 · research §C4): the off-screen export host injects content
  // through this and awaits the returned promise, so Mermaid's async layout is
  // complete before NSPrintOperation paginates. KaTeX renders synchronously.
  window.__emendRenderForPrint = async function (html, css) {
    injectThemeCSS(css);
    var content = document.getElementById("emend-content");
    if (!content) return;
    content.innerHTML = html;
    await renderMermaid(content);
    renderMath(content);
  };
})();
