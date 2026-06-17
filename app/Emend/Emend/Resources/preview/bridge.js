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
        window.mermaid.run({ nodes: root.querySelectorAll(".mermaid") });
      } catch (e) {
        /* malformed diagram — leave the source visible, never throw */
      }
    }
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

  window.__emendRender = function (html, css) {
    injectThemeCSS(css);
    var content = document.getElementById("emend-content");
    if (!content) return;
    content.innerHTML = html;
    renderMermaid(content);
    renderMath(content);
    // Scroll-sync anchor table is (re)built here once T088 wires it in.
    if (typeof window.__emendAfterRender === "function") {
      window.__emendAfterRender();
    }
  };
})();
