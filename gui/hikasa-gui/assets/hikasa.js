// Hikasa page script: the canvas text views (ARCH-GUI-002), the folder tree scroll helper and the scripted
// benchmark drivers (ARCH-GUI-003). Rows come from the Rust side as slices ("/hk/slice"); the page keeps a
// bounded cache of them and draws only what is on screen. No DOM node exists per line.
(function () {
  'use strict';
  if (window.hikasa) {
    return;
  }
  const H = {};
  window.hikasa = H;

  const MAX_ROWS = 6144;
  const views = new Map();

  function byId(id) {
    return document.getElementById(id);
  }

  function token(name) {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  let theme = null;
  let themeVersion = 0;
  function loadTheme() {
    themeVersion += 1;
    const syn = [token('--hk-fg')];
    for (let i = 1; i <= 8; i++) {
      syn.push(token('--hk-syn-' + i));
    }
    theme = {
      bg: token('--hk-bg'),
      fg: token('--hk-fg'),
      muted: token('--hk-muted'),
      border: token('--hk-border'),
      gutter: token('--hk-gutter'),
      gutterFg: token('--hk-gutter-fg'),
      impBg: token('--hk-imp-bg'),
      impHl: token('--hk-imp-hl'),
      unimpBg: token('--hk-unimp-bg'),
      unimpHl: token('--hk-unimp-hl'),
      orphImpBg: token('--hk-orph-imp-bg'),
      orphUnimpBg: token('--hk-orph-unimp-bg'),
      gap: token('--hk-gap'),
      gapFg: token('--hk-gap-fg'),
      hatch: token('--hk-hatch'),
      cursor: token('--hk-cursor'),
      thumbImp: token('--hk-thumb-imp'),
      thumbUnimp: token('--hk-thumb-unimp'),
      thumbOrph: token('--hk-thumb-orph'),
      thumbView: token('--hk-thumb-view'),
      syn: syn,
    };
  }

  if (window.matchMedia) {
    window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', function () {
      loadTheme();
      for (const v of views.values()) {
        v.hatch = null;
        v.thumbImage = null;
        v.thumbGen = -1;
        v.request();
      }
    });
  }

  async function rpc(op, params, body) {
    const query = new URLSearchParams(params).toString();
    const init = body === undefined ? {} : { method: 'POST', body: body };
    const res = await fetch('/hk/' + op + '?' + query, init);
    if (res.status === 204) {
      return null;
    }
    if (!res.ok) {
      throw new Error(op + ' failed: ' + res.status);
    }
    return res.json();
  }

  function nextFrame() {
    return new Promise(function (resolve) {
      requestAnimationFrame(resolve);
    });
  }

  function clamp(x, lo, hi) {
    return Math.max(lo, Math.min(hi, x));
  }

  // ----------------------------------------------------------------------------------------------------------
  // Text view
  // ----------------------------------------------------------------------------------------------------------

  class TextView {
    constructor(id) {
      this.id = id;
      this.meta = null;
      this.gen = -1;
      this.cache = new Map();
      this.inflight = new Set();
      this.lastFirst = 0;
      this.lastVelocity = 0;
      this.top = 0;
      this.left = 0;
      this.cursor = -1;
      this.raf = 0;
      this.width = 0;
      this.height = 0;
      this.dpr = 1;
      this.cw = 8;
      this.fontKey = '';
      this.hatch = null;
      this.thumbImage = null;
      this.thumbGen = -1;
      this.thumbHeight = 0;
      this.thumbBusy = false;
      this.drawnPhase = 0;
      this.restored = false;
      this.reportTimer = 0;
      this.reported = '';
      this.fetches = 0;
      this.fetchedRows = 0;
      this.fetchMs = 0;
      this.listeners = [];
      this.observer = null;
      this.drag = null;
      this.last = null;
    }

    els() {
      const id = this.id;
      return {
        body: byId('hk-tb-' + id),
        canvas: byId('hk-tc-' + id),
        thumb: byId('hk-tt-' + id),
        track: byId('hk-tv-' + id),
        knob: byId('hk-tk-' + id),
      };
    }

    get lh() {
      const font = this.meta ? this.meta.font : 13;
      return Math.round(font * 1.45);
    }

    get len() {
      return this.meta ? this.meta.len : 0;
    }

    get maxTop() {
      return Math.max(0, this.len * this.lh - this.height);
    }

    listen(el, type, fn, opts) {
      if (!el) {
        return;
      }
      el.addEventListener(type, fn, opts);
      this.listeners.push([el, type, fn, opts]);
    }

    mount() {
      const e = this.els();
      if (!e.canvas || !e.body) {
        return false;
      }
      this.unmount();
      const self = this;
      this.observer = new ResizeObserver(function () {
        self.resize();
      });
      this.observer.observe(e.body);
      this.listen(e.canvas, 'wheel', function (ev) {
        ev.preventDefault();
        const unit = ev.deltaMode === 1 ? self.lh : ev.deltaMode === 2 ? self.height : 1;
        let dx = ev.deltaX * unit;
        let dy = ev.deltaY * unit;
        if (ev.shiftKey && dx === 0) {
          dx = dy;
          dy = 0;
        }
        self.scrollBy(dy, dx);
      }, { passive: false });
      this.listen(e.canvas, 'mousedown', function (ev) {
        e.canvas.focus();
        const rect = e.canvas.getBoundingClientRect();
        const pos = Math.floor((self.top + ev.clientY - rect.top) / self.lh);
        if (pos >= 0 && pos < self.len) {
          self.click(pos);
        }
      });
      this.listen(e.canvas, 'keydown', function (ev) {
        if (ev.ctrlKey || ev.altKey || ev.metaKey) {
          if ((ev.key === 'Home' || ev.key === 'End') && ev.ctrlKey) {
            ev.preventDefault();
            self.scrollTo(ev.key === 'Home' ? 0 : self.maxTop);
          }
          return;
        }
        const page = Math.max(self.lh, self.height - self.lh);
        const moves = {
          ArrowDown: [self.lh, 0],
          ArrowUp: [-self.lh, 0],
          PageDown: [page, 0],
          PageUp: [-page, 0],
          ArrowRight: [0, self.cw * 4],
          ArrowLeft: [0, -self.cw * 4],
        };
        const m = moves[ev.key];
        if (m) {
          ev.preventDefault();
          self.scrollBy(m[0], m[1]);
        } else if (ev.key === 'Home' || ev.key === 'End') {
          ev.preventDefault();
          self.scrollTo(ev.key === 'Home' ? 0 : self.maxTop);
        }
      });
      this.listen(e.thumb, 'mousedown', function (ev) {
        ev.preventDefault();
        self.thumbJump(ev);
      });
      this.listen(e.knob, 'mousedown', function (ev) {
        ev.preventDefault();
        ev.stopPropagation();
        self.drag = { y: ev.clientY, top: self.top };
      });
      this.listen(e.track, 'mousedown', function (ev) {
        if (ev.target !== e.track) {
          return;
        }
        const rect = e.track.getBoundingClientRect();
        const knobTop = self.knobTop();
        const page = Math.max(self.lh, self.height - self.lh);
        self.scrollBy(ev.clientY - rect.top < knobTop ? -page : page, 0);
      });
      this.listen(window, 'mousemove', function (ev) {
        if (!self.drag) {
          return;
        }
        const e2 = self.els();
        if (!e2.track) {
          return;
        }
        const travel = Math.max(1, e2.track.clientHeight - self.knobHeight());
        self.scrollTo(self.drag.top + (ev.clientY - self.drag.y) * (self.maxTop / travel));
      });
      this.listen(window, 'mouseup', function () {
        self.drag = null;
      });
      this.resize();
      return true;
    }

    unmount() {
      for (const [el, type, fn, opts] of this.listeners) {
        el.removeEventListener(type, fn, opts);
      }
      this.listeners = [];
      if (this.observer) {
        this.observer.disconnect();
        this.observer = null;
      }
    }

    resize() {
      const e = this.els();
      if (!e.canvas) {
        return;
      }
      this.dpr = window.devicePixelRatio || 1;
      const dpr = this.dpr;
      // The canvas is sized to a whole number of device pixels, so its buffer is shown 1:1: a CSS size that
      // is not a multiple of 1/dpr makes the compositor rescale the canvas on every frame (measured: 25 fps
      // instead of 60 at dpr 1.5).
      const fit = function (el) {
        el.style.width = '';
        el.style.height = '';
        const box = el.getBoundingClientRect();
        const w = Math.max(1, Math.floor(box.width * dpr));
        const h = Math.max(1, Math.floor(box.height * dpr));
        el.style.width = w / dpr + 'px';
        el.style.height = h / dpr + 'px';
        el.width = w;
        el.height = h;
        return [w / dpr, h / dpr];
      };
      [this.width, this.height] = fit(e.canvas);
      if (e.thumb) {
        fit(e.thumb);
        if (e.thumb.clientHeight !== this.thumbHeight) {
          this.thumbGen = -1;
        }
      }
      this.fontKey = '';
      this.last = null;
      this.request();
    }

    async refresh() {
      try {
        const meta = await rpc('meta', { view: this.id });
        if (meta) {
          this.setMeta(meta);
        }
      } catch (err) {
        console.warn(err);
      }
    }

    setMeta(meta) {
      const lhBefore = this.meta ? this.lh : 0;
      if (meta.gen !== this.gen) {
        this.cache.clear();
        this.inflight.clear();
        this.gen = meta.gen;
        this.thumbGen = -1;
      }
      this.meta = meta;
      if (lhBefore && lhBefore !== this.lh) {
        this.top = (this.top / lhBefore) * this.lh;
        this.fontKey = '';
      }
      if (!this.restored) {
        this.restored = true;
        this.top = meta.top * this.lh;
      }
      this.cursor = meta.cursor;
      if (meta.scroll >= 0) {
        this.reveal(meta.scroll);
      }
      const e = this.els();
      if (e.thumb) {
        e.thumb.style.visibility = meta.thumb ? 'visible' : 'hidden';
      }
      this.top = clamp(this.top, 0, this.maxTop);
      this.request();
    }

    reveal(pos) {
      const y = pos * this.lh;
      if (y < this.top || y + this.lh > this.top + this.height) {
        this.top = clamp(y - this.height / 3, 0, this.maxTop);
      }
    }

    scrollBy(dy, dx) {
      this.top = clamp(this.top + dy, 0, this.maxTop);
      this.left = clamp(this.left + dx, 0, 4096 * this.cw);
      this.request();
    }

    scrollTo(top) {
      this.top = clamp(top, 0, this.maxTop);
      this.request();
    }

    async click(pos) {
      this.cursor = pos;
      this.request();
      try {
        const r = await rpc('click', { view: this.id, pos: pos });
        if (r && r.changed) {
          await this.refresh();
        }
      } catch (err) {
        console.warn(err);
      }
    }

    async thumbJump(ev) {
      const e = this.els();
      if (!e.thumb || !this.meta || !this.meta.rows) {
        return;
      }
      const rect = e.thumb.getBoundingClientRect();
      const row = Math.floor(clamp((ev.clientY - rect.top) / rect.height, 0, 1) * (this.meta.rows - 1));
      try {
        const r = await rpc('thumbclick', { view: this.id, row: row });
        if (r) {
          this.scrollTo(r.pos * this.lh - this.height / 3);
        }
      } catch (err) {
        console.warn(err);
      }
    }

    request() {
      if (this.raf) {
        return;
      }
      const self = this;
      this.raf = requestAnimationFrame(function () {
        self.raf = 0;
        self.frame();
      });
    }

    visibleRange() {
      const geo = this.geometry();
      return [geo.first, geo.count];
    }

    // A request for rows [start, start + count); the rows land in the row cache.
    fetchRange(start, count) {
      const gen = this.gen;
      const self = this;
      const began = performance.now();
      const p = rpc('slice', { view: this.id, start: start, count: count })
        .then(function (slice) {
          self.inflight.delete(p);
          self.fetches += 1;
          self.fetchMs += performance.now() - began;
          if (!slice) {
            return;
          }
          if (slice.gen !== self.gen || gen !== self.gen) {
            // Stale data: the view changed meanwhile.
            return self.refresh();
          }
          const rows = slice.rows;
          for (let i = 0; i < rows.length; i++) {
            self.cache.set(slice.start + i, rows[i]);
          }
          self.fetchedRows += rows.length;
          self.evict(slice.start);
        })
        .catch(function (err) {
          self.inflight.delete(p);
          console.warn(err);
        });
      p.range = [start, start + count];
      this.inflight.add(p);
      return p;
    }

    covering(pos) {
      for (const p of this.inflight) {
        if (p.range[0] <= pos && pos < p.range[1]) {
          return p;
        }
      }
      return null;
    }

    // Keeps the rows near `near` when the cache grows past its bound.
    evict(near) {
      if (this.cache.size <= MAX_ROWS) {
        return;
      }
      const lo = near - MAX_ROWS / 2;
      const hi = near + MAX_ROWS / 2;
      for (const pos of this.cache.keys()) {
        if (pos < lo || pos >= hi) {
          this.cache.delete(pos);
        }
      }
    }

    row(pos) {
      return this.cache.get(pos);
    }

    // Requests the rows the screen needs, in one request (extended by a page in the scroll direction when
    // scrolling smoothly); when everything is there, prefetches the next page.
    ensure() {
      const [first, count] = this.visibleRange();
      const waits = new Set();
      let lo = -1;
      let hi = -1;
      for (let pos = first; pos < first + count; pos++) {
        if (this.cache.has(pos)) {
          continue;
        }
        const p = this.covering(pos);
        if (p) {
          waits.add(p);
        } else {
          if (lo < 0) {
            lo = pos;
          }
          hi = pos + 1;
        }
      }
      const velocity = first - this.lastFirst;
      const down = velocity >= 0;
      const smooth = Math.abs(velocity) <= count;
      const steady = !smooth && Math.abs(velocity - this.lastVelocity) <= count;
      this.lastFirst = first;
      this.lastVelocity = velocity;
      if (lo >= 0) {
        let start = lo;
        let end = hi;
        if (smooth && down) {
          end = Math.min(this.len, hi + count);
        } else if (smooth) {
          start = Math.max(0, lo - count);
        }
        waits.add(this.fetchRange(start, end - start));
      } else if (!waits.size && smooth) {
        // The first row of the next page that is not there yet, fetched with two pages beyond it.
        const ahead = 2 * count;
        if (down) {
          const end = Math.min(this.len, first + 2 * count);
          for (let pos = first + count; pos < end; pos++) {
            if (!this.cache.has(pos) && !this.covering(pos)) {
              this.fetchRange(pos, Math.min(this.len, pos + ahead) - pos);
              break;
            }
          }
        } else {
          for (let pos = first - 1; pos >= Math.max(0, first - count); pos--) {
            if (!this.cache.has(pos) && !this.covering(pos)) {
              const start = Math.max(0, pos + 1 - ahead);
              this.fetchRange(start, pos + 1 - start);
              break;
            }
          }
        }
      }
      // Fast, steady scrolling (a dragged scroll bar): fetch where the next frame will be, so that its rows are
      // on their way while this frame is drawn.
      if (steady) {
        // A margin covers the rounding of the next position.
        const margin = Math.max(2, Math.ceil(count / 4));
        const next = clamp(first + velocity - margin, 0, Math.max(0, this.len - count));
        const end = Math.min(this.len, next + count + 2 * margin);
        for (let pos = next; pos < end; pos++) {
          if (!this.cache.has(pos) && !this.covering(pos)) {
            this.fetchRange(pos, end - pos);
            break;
          }
        }
      }
      return Array.from(waits);
    }

    frame() {
      if (!this.meta) {
        return;
      }
      const missing = this.ensure();
      this.paint();
      if (missing.length) {
        const self = this;
        Promise.all(missing).then(function () {
          self.request();
        });
      } else {
        this.afterPaint();
      }
      this.thumbnail();
    }

    // Draws once everything on screen is loaded (the benchmark's frame).
    async drawComplete() {
      const began = performance.now();
      for (let i = 0; i < 4; i++) {
        const missing = this.ensure();
        if (!missing.length) {
          break;
        }
        await Promise.all(missing);
      }
      const loaded = performance.now();
      this.paint();
      this.afterPaint();
      return [loaded - began, performance.now() - loaded];
    }

    afterPaint() {
      const m = this.meta;
      let phase = 0;
      if (m.phase === 1) {
        phase = 1;
      } else if (m.phase === 2) {
        phase = m.syntax ? 2 : 1;
      }
      if (phase > this.drawnPhase && this.len > 0) {
        this.drawnPhase = phase;
        rpc('drawn', { view: this.id, phase: phase }).catch(function () {});
      }
      const [first, count] = this.visibleRange();
      const key = first + ':' + count;
      if (key !== this.reported && !this.reportTimer) {
        const self = this;
        this.reportTimer = setTimeout(function () {
          self.reportTimer = 0;
          const [f, c] = self.visibleRange();
          self.reported = f + ':' + c;
          rpc('viewport', { view: self.id, top: f, page: c }).catch(function () {});
        }, 150);
      }
    }

    hatchPattern(ctx) {
      if (this.hatch) {
        return this.hatch;
      }
      const c = document.createElement('canvas');
      c.width = 8;
      c.height = 8;
      const g = c.getContext('2d');
      g.strokeStyle = theme.hatch;
      g.lineWidth = 1;
      g.beginPath();
      g.moveTo(0, 8);
      g.lineTo(8, 0);
      g.stroke();
      this.hatch = ctx.createPattern(c, 'repeat');
      return this.hatch;
    }

    // Screen geometry in device pixels (rows on whole device pixels, so a scroll can reuse drawn rows).
    geometry() {
      const dpr = this.dpr;
      const lhD = Math.max(1, Math.round(this.lh * dpr));
      const topD = Math.round(this.top * dpr);
      const Hd = Math.max(1, Math.round(this.height * dpr));
      const Wd = Math.max(1, Math.round(this.width * dpr));
      const first = Math.floor(topD / lhD);
      const offD = topD - first * lhD;
      const count = Math.max(0, Math.min(Math.ceil((Hd + offD) / lhD), this.len - first));
      return { dpr: dpr, lhD: lhD, topD: topD, Hd: Hd, Wd: Wd, first: first, offD: offD, count: count };
    }

    // Draws the visible rows. When only the scroll position changed since the last complete frame, the
    // drawn rows are moved and only the uncovered band is drawn.
    paint() {
      const e = this.els();
      if (!e.canvas || !this.meta) {
        return;
      }
      if (!theme) {
        loadTheme();
      }
      const ctx = e.canvas.getContext('2d', { alpha: false });
      const m = this.meta;
      const geo = this.geometry();
      const dpr = geo.dpr;
      const lhD = geo.lhD;
      const Wd = geo.Wd;
      const Hd = geo.Hd;
      const first = geo.first;
      const count = geo.count;
      const offD = geo.offD;
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      const font = Math.round(m.font * dpr) + 'px ui-monospace, "DejaVu Sans Mono", "Liberation Mono", "Noto Mono", monospace';
      if (this.fontKey !== font) {
        this.fontKey = font;
        ctx.font = font;
        this.cw = ctx.measureText('MMMMMMMMMM').width / 10;
        this.last = null;
      }
      ctx.textBaseline = 'middle';
      const cw = this.cw;
      const divW = Math.max(1, Math.round(dpr));
      const paneW = Math.floor((Wd - divW) / 2);
      const gutterW = Math.ceil((m.digits + 1.5) * cw);
      const pad = Math.round(4 * dpr);
      const leftD = Math.round(this.left * dpr);
      const key = [this.gen, leftD, Wd, Hd, font, this.cursor, m.digits, themeVersion].join(',');
      let band0 = 0;
      let band1 = Hd;
      let shifted = false;
      const last = this.last;
      if (last && last.key === key && last.complete) {
        const dy = geo.topD - last.topD;
        if (dy === 0) {
          this.knob();
          this.thumbViewport(first, count);
          return;
        }
        if (Math.abs(dy) < Hd / 2) {
          if (dy > 0) {
            ctx.drawImage(e.canvas, 0, dy, Wd, Hd - dy, 0, 0, Wd, Hd - dy);
            band0 = Hd - dy;
          } else {
            ctx.drawImage(e.canvas, 0, 0, Wd, Hd + dy, 0, -dy, Wd, Hd + dy);
            band1 = -dy;
          }
          shifted = true;
        }
      }
      ctx.save();
      if (shifted) {
        ctx.beginPath();
        ctx.rect(0, band0, Wd, band1 - band0);
        ctx.clip();
      }
      ctx.fillStyle = theme.bg;
      ctx.fillRect(0, band0, Wd, band1 - band0);
      if (!this.len) {
        ctx.fillStyle = theme.muted;
        ctx.textAlign = 'center';
        ctx.fillText(m.message || '', Wd / 2, Math.min(Hd / 2, 60 * dpr));
        ctx.textAlign = 'left';
        ctx.restore();
        this.last = null;
        this.knob();
        return;
      }
      const hatch = this.hatchPattern(ctx);
      const batches = [new Map(), new Map()];
      const numbers = [];
      const firstCol = Math.floor(leftD / cw);
      const lastCol = firstCol + Math.ceil((paneW - gutterW) / cw) + 1;
      let complete = true;
      for (let i = 0; i < count; i++) {
        const y = i * lhD - offD;
        if (y + lhD <= band0 || y >= band1) {
          continue;
        }
        const r = this.row(first + i);
        if (!r) {
          complete = false;
          ctx.fillStyle = theme.gap;
          ctx.fillRect(0, y + 2 * dpr, Wd, lhD - 4 * dpr);
          continue;
        }
        const mid = y + lhD / 2;
        if (r.k === 1) {
          ctx.fillStyle = theme.gap;
          ctx.fillRect(0, y, Wd, lhD);
          ctx.fillStyle = theme.gapFg;
          ctx.fillText('▸ ' + r.g.toLocaleString() + (r.g === 1 ? ' hidden line' : ' hidden lines'), gutterW + pad, mid);
          continue;
        }
        for (let side = 0; side < 2; side++) {
          const x0 = side * (paneW + divW);
          const text = r.t[side];
          let bg = null;
          if (text === null) {
            bg = hatch;
          } else if (r.st === 1) {
            bg = r.im === 2 ? theme.impBg : theme.unimpBg;
          } else if (r.st === 2 || r.st === 3) {
            bg = r.im === 2 ? theme.orphImpBg : theme.orphUnimpBg;
          }
          ctx.fillStyle = theme.gutter;
          ctx.fillRect(x0, y, gutterW, lhD);
          if (bg) {
            ctx.fillStyle = bg;
            ctx.fillRect(x0 + gutterW, y, paneW - gutterW, lhD);
          }
          if (text === null) {
            continue;
          }
          numbers.push(x0 + gutterW - cw * 0.5, mid, r.ln[side]);
          const tx = x0 + gutterW + pad - leftD;
          const hl = r.hl[side];
          for (let k = 0; k + 2 < hl.length; k += 3) {
            const s = hl[k];
            const end = hl[k + 1];
            ctx.fillStyle = hl[k + 2] ? theme.impHl : theme.unimpHl;
            const x = Math.max(tx + s * cw, x0 + gutterW);
            const w = Math.max(2 * dpr, (end - s) * cw - (x - (tx + s * cw)));
            ctx.fillRect(x, y + dpr, w, lhD - 2 * dpr);
          }
          // One run per colour and line: the other colours' characters become spaces.
          const sp = r.sp[side];
          const parts = [];
          for (let k = 0; k + 2 < sp.length; k += 3) {
            const s = Math.max(sp[k], firstCol);
            const end = Math.min(sp[k + 1], lastCol);
            if (end <= s) {
              continue;
            }
            const colour = sp[k + 2];
            let part = parts[colour];
            if (!part) {
              part = parts[colour] = [s, ''];
            }
            const gap = s - (part[0] + part[1].length);
            if (gap > 0) {
              part[1] += ' '.repeat(gap);
            }
            part[1] += text.substring(s, end);
          }
          const batch = batches[side];
          for (let colour = 0; colour < parts.length; colour++) {
            const part = parts[colour];
            if (!part) {
              continue;
            }
            let list = batch.get(colour);
            if (!list) {
              list = [];
              batch.set(colour, list);
            }
            list.push(tx + part[0] * cw, mid, part[1]);
          }
        }
      }
      // Text runs, batched per pane and colour (one fillStyle change per colour).
      for (let side = 0; side < 2; side++) {
        const x0 = side * (paneW + divW);
        ctx.save();
        ctx.beginPath();
        ctx.rect(x0 + gutterW, band0, paneW - gutterW, band1 - band0);
        ctx.clip();
        for (const [colour, list] of batches[side]) {
          ctx.fillStyle = theme.syn[colour] || theme.fg;
          for (let k = 0; k < list.length; k += 3) {
            ctx.fillText(list[k + 2], list[k], list[k + 1]);
          }
        }
        ctx.restore();
      }
      ctx.fillStyle = theme.gutterFg;
      ctx.textAlign = 'right';
      for (let k = 0; k < numbers.length; k += 3) {
        if (numbers[k + 2] > 0) {
          ctx.fillText(String(numbers[k + 2]), numbers[k], numbers[k + 1]);
        }
      }
      ctx.textAlign = 'left';
      ctx.fillStyle = theme.border;
      ctx.fillRect(paneW, band0, divW, band1 - band0);
      if (this.cursor >= first && this.cursor < first + count) {
        const y = (this.cursor - first) * lhD - offD;
        ctx.strokeStyle = theme.cursor;
        ctx.lineWidth = divW;
        ctx.strokeRect(divW / 2, y + divW / 2, Wd - divW, lhD - divW);
      }
      ctx.restore();
      this.last = { key: key, topD: geo.topD, complete: complete };
      this.knob();
      this.thumbViewport(first, count);
    }

    knobHeight() {
      const e = this.els();
      if (!e.track) {
        return 18;
      }
      const total = Math.max(1, this.len * this.lh);
      return Math.max(18, Math.min(e.track.clientHeight, e.track.clientHeight * (this.height / total)));
    }

    knobTop() {
      const e = this.els();
      if (!e.track) {
        return 0;
      }
      const travel = e.track.clientHeight - this.knobHeight();
      return this.maxTop > 0 ? travel * (this.top / this.maxTop) : 0;
    }

    knob() {
      const e = this.els();
      if (!e.knob) {
        return;
      }
      e.knob.style.height = this.knobHeight() + 'px';
      e.knob.style.transform = 'translateY(' + this.knobTop() + 'px)';
    }

    async thumbnail() {
      const e = this.els();
      if (!e.thumb || !this.meta || !this.meta.thumb || this.meta.phase !== 2) {
        return;
      }
      const h = e.thumb.clientHeight;
      if ((this.thumbGen === this.gen && this.thumbHeight === h) || this.thumbBusy || h <= 0) {
        return;
      }
      this.thumbBusy = true;
      try {
        const data = await rpc('thumb', { view: this.id, h: Math.round(h) });
        if (data && data.gen === this.gen) {
          const img = document.createElement('canvas');
          img.width = e.thumb.width;
          img.height = e.thumb.height;
          const g = img.getContext('2d');
          const scale = img.height / Math.max(1, data.cells.length);
          for (let i = 0; i < data.cells.length; i++) {
            const c = data.cells[i];
            let colour = null;
            if (c & 4) {
              colour = theme.thumbImp;
            } else if (c & 24) {
              colour = theme.thumbOrph;
            } else if (c & 2) {
              colour = theme.thumbUnimp;
            }
            if (colour) {
              g.fillStyle = colour;
              g.fillRect(0, Math.floor(i * scale), img.width, Math.max(1, Math.ceil(scale)));
            }
          }
          this.thumbImage = img;
          this.thumbGen = data.gen;
          this.thumbHeight = h;
          this.request();
        }
      } catch (err) {
        console.warn(err);
      } finally {
        this.thumbBusy = false;
      }
    }

    thumbViewport(first, count) {
      const e = this.els();
      if (!e.thumb || !this.meta || !this.meta.thumb) {
        return;
      }
      const g = e.thumb.getContext('2d');
      g.setTransform(1, 0, 0, 1, 0, 0);
      g.fillStyle = theme.bg;
      g.fillRect(0, 0, e.thumb.width, e.thumb.height);
      if (this.thumbImage) {
        g.drawImage(this.thumbImage, 0, 0);
      }
      const rows = this.meta.rows || this.len;
      if (!rows) {
        return;
      }
      const a = this.row(first);
      const b = this.row(Math.max(first, first + count - 2));
      const r0 = a ? a.r : first;
      const r1 = b ? b.r + Math.max(1, b.g) : first + count;
      const y0 = (r0 / rows) * e.thumb.height;
      const y1 = Math.max(y0 + 2 * this.dpr, (r1 / rows) * e.thumb.height);
      g.fillStyle = theme.thumbView;
      g.fillRect(0, y0, e.thumb.width, y1 - y0);
    }
  }

  H.textMount = function (id) {
    let v = views.get(id);
    if (!v) {
      v = new TextView(id);
      views.set(id, v);
    }
    // The canvas may be attached a moment after the container.
    const tryMount = function (attempt) {
      if (v.mount()) {
        v.refresh();
        rpc('drawn', { view: id, phase: 0 }).catch(function () {});
      } else if (attempt < 20) {
        requestAnimationFrame(function () {
          tryMount(attempt + 1);
        });
      }
    };
    tryMount(0);
  };

  H.textUnmount = function (id) {
    const v = views.get(id);
    if (v) {
      v.unmount();
      views.delete(id);
    }
  };

  H.textDirty = function (id) {
    const v = views.get(id);
    if (v) {
      v.refresh();
    }
  };

  // ----------------------------------------------------------------------------------------------------------
  // Folder tree
  // ----------------------------------------------------------------------------------------------------------

  H.folderScrollTo = function (id, index, rowHeight) {
    const el = byId('hk-fs-' + id);
    if (!el) {
      return;
    }
    const y = index * rowHeight;
    if (y < el.scrollTop || y + rowHeight > el.scrollTop + el.clientHeight) {
      el.scrollTop = Math.max(0, y - el.clientHeight / 3);
    }
  };

  // ----------------------------------------------------------------------------------------------------------
  // Benchmarks (ARCH-GUI-003)
  // ----------------------------------------------------------------------------------------------------------

  function stats(samples) {
    if (!samples.length) {
      return { count: 0 };
    }
    const s = samples.slice().sort(function (a, b) {
      return a - b;
    });
    const pick = function (q) {
      return s[Math.round((s.length - 1) * q)];
    };
    const r = function (x) {
      return Math.round(x * 100) / 100;
    };
    let sum = 0;
    let over = 0;
    let missed = 0;
    for (const x of s) {
      sum += x;
      if (x > 1000 / 60) {
        over += 1;
      }
      if (x > 25) {
        missed += 1;
      }
    }
    return {
      count: s.length,
      mean: r(sum / s.length),
      p50: r(pick(0.5)),
      p95: r(pick(0.95)),
      p99: r(pick(0.99)),
      max: r(s[s.length - 1]),
      over_16_7ms: over,
      // An interval above 1.5 frames is a missed frame (the page clock has 1 ms resolution).
      over_25ms: missed,
    };
  }

  async function postResults(id, results) {
    try {
      await fetch('/hk/bench?view=' + id, { method: 'POST', body: JSON.stringify(results) });
    } catch (err) {
      console.warn(err);
    }
  }

  async function scrollPhase(v, name, frames, step, wrap) {
    const frameTimes = [];
    const intervals = [];
    const waits = [];
    const paints = [];
    const fetchesBefore = v.fetches;
    const rowsBefore = v.fetchedRows;
    let last = await nextFrame();
    const t0 = performance.now();
    for (let i = 0; i < frames; i++) {
      const ts = await nextFrame();
      intervals.push(ts - last);
      last = ts;
      const began = performance.now();
      let top = v.top + step;
      if (top > v.maxTop + 0.5) {
        top = wrap ? 0 : v.maxTop;
      }
      v.top = clamp(top, 0, v.maxTop);
      const [wait, paint] = await v.drawComplete();
      waits.push(wait);
      paints.push(paint);
      frameTimes.push(performance.now() - began);
    }
    const elapsed = performance.now() - t0;
    return {
      name: name,
      frames: frames,
      step_px: Math.round(step * 100) / 100,
      seconds: Math.round(elapsed) / 1000,
      fps: Math.round((frames / (elapsed / 1000)) * 10) / 10,
      frame_work_ms: stats(frameTimes),
      frame_interval_ms: stats(intervals),
      data_wait_ms: stats(waits),
      paint_ms: stats(paints),
      slice_requests: v.fetches - fetchesBefore,
      rows_fetched: v.fetchedRows - rowsBefore,
    };
  }

  H.benchScroll = async function (id) {
    const v = views.get(id);
    if (!v || !v.meta) {
      await postResults(id, { error: 'no text view' });
      return;
    }
    await nextFrame();
    const lh = v.lh;
    const total = v.len * lh;
    const results = {
      positions: v.len,
      viewport_px: [v.width, v.height],
      device_pixel_ratio: v.dpr,
      line_height_px: lh,
      visible_rows: Math.ceil(v.height / lh),
      phases: [],
    };
    v.top = 0;
    results.phases.push(await scrollPhase(v, 'wheel_3_lines_per_frame', 600, 3 * lh, true));
    results.phases.push(await scrollPhase(v, 'page_per_frame', 300, Math.max(lh, v.height - lh), true));
    v.top = 0;
    const sweepFrames = 1200;
    results.phases.push(await scrollPhase(v, 'sweep_whole_comparison', sweepFrames, Math.max(lh, (total - v.height) / sweepFrames), false));
    await postResults(id, results);
  };

  H.benchFolder = function (id) {
    const state = { intervals: [], running: true, t0: performance.now(), scrolled: 0 };
    H._folderBench = { id: id, state: state };
    let last = performance.now();
    const loop = function (ts) {
      if (!state.running) {
        return;
      }
      state.intervals.push(ts - last);
      last = ts;
      const el = byId('hk-fs-' + id);
      if (el) {
        const before = el.scrollTop;
        el.scrollTop = before + 44;
        if (el.scrollTop !== before) {
          state.scrolled += 1;
        }
      }
      requestAnimationFrame(loop);
    };
    requestAnimationFrame(function (ts) {
      last = ts;
      requestAnimationFrame(loop);
    });
  };

  // The scan finished: the frames so far are the ones drawn while scanning.
  H.benchFolderScanDone = function () {
    const b = H._folderBench;
    if (b && b.state.scanFrames === undefined) {
      b.state.scanFrames = b.state.intervals.length;
    }
  };

  H.benchFolderStop = function () {
    const b = H._folderBench;
    if (!b) {
      return;
    }
    const s = b.state;
    s.running = false;
    const elapsed = performance.now() - s.t0;
    postResults(b.id, {
      frames: s.intervals.length,
      seconds: Math.round(elapsed) / 1000,
      fps: Math.round((s.intervals.length / (elapsed / 1000)) * 10) / 10,
      frame_interval_ms: stats(s.intervals),
      frame_interval_ms_while_scanning: stats(s.intervals.slice(0, s.scanFrames === undefined ? s.intervals.length : s.scanFrames)),
      scroll_steps: s.scrolled,
    });
  };
})();
