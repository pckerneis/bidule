/* =====================================================================
   Audio: 4 synth voices (Sound records) + optional bytebeat audio(t)
   ===================================================================== */
let actx = null, master = null, noiseBuf = null, muted = false;
let bbSample = 0, bbLastT = -1, bbVal = 128;
const voices = [0, 1, 2, 3].map(() => ({ start: 0, end: 0, nodes: null }));
const nowMs = () => performance.now();

function ensureAudio() {
  try {
    if (!actx) {
      const AC = window.AudioContext || window.webkitAudioContext;
      if (!AC) return;
      actx = new AC();
      master = actx.createGain(); master.gain.value = muted ? 0 : 1; master.connect(actx.destination);
      noiseBuf = actx.createBuffer(1, actx.sampleRate, actx.sampleRate);
      const d = noiseBuf.getChannelData(0); for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
      const sp = actx.createScriptProcessor(1024, 0, 1);
      sp.onaudioprocess = bytebeatFill;
      sp.connect(master);
    }
    if (actx.state === "suspended") actx.resume();
  } catch (e) { /* audio unavailable in this viewer */ }
}
function bytebeatFill(e) {
  const out = e.outputBuffer.getChannelData(0);
  if (!vm || !vm.hasAudio || halted || muted) { out.fill(0); return; }
  const rate = actx.sampleRate;
  for (let i = 0; i < out.length; i++) {
    const t = Math.floor(bbSample * 8000 / rate); bbSample++;
    if (t !== bbLastT) { bbLastT = t; bbVal = vm.audioSample(t | 0); }
    out[i] = (bbVal - 128) / 128 * 0.12;
  }
}
const SEMI = { C: -9, D: -7, E: -5, F: -4, G: -2, A: 0, B: 2 };
function noteFreq(n) {
  if (n === "-") return null;
  const m = /^([A-Ga-g])([#b]?)(-?\d)$/.exec(n);
  if (!m) return undefined;
  const s = SEMI[m[1].toUpperCase()] + (m[2] === "#" ? 1 : m[2] === "b" ? -1 : 0) + (+m[3] - 4) * 12;
  return 440 * Math.pow(2, s / 12);
}
const pulseCache = new Map();
function pulseWave(d) {
  d = Math.max(0.02, Math.min(0.98, d));
  const key = Math.round(d * 256);
  if (pulseCache.has(key)) return pulseCache.get(key);
  const N = 48, re = new Float32Array(N), im = new Float32Array(N);
  for (let n = 1; n < N; n++) re[n] = 2 * Math.sin(Math.PI * n * d) / (Math.PI * n);
  const w = actx.createPeriodicWave(re, im);
  pulseCache.set(key, w); return w;
}
function stopVoice(v) {
  const vo = voices[v];
  if (vo.nodes) { try { vo.nodes.src.stop(); } catch (_) {} try { vo.nodes.g.disconnect(); } catch (_) {} }
  vo.nodes = null; vo.end = 0;
}
function stopAllVoices() { for (let v = 0; v < 4; v++) stopVoice(v); }
function startSound(snd, v) {
  const dur = snd.seq ? snd.seq.length * snd.step : snd.time;
  const vo = voices[v];
  vo.start = nowMs(); vo.end = vo.start + (dur + snd.release) * 1000;
  if (!actx || actx.state !== "running") return;
  try {
    const t = actx.currentTime + 0.005;
    const g = actx.createGain();
    let src, freqParam;
    if (snd.wave === 3) {
      src = actx.createBufferSource(); src.buffer = noiseBuf; src.loop = true;
      const bp = actx.createBiquadFilter(); bp.type = "bandpass"; bp.Q.value = 0.8;
      src.connect(bp); bp.connect(g); freqParam = bp.frequency;
    } else {
      src = actx.createOscillator(); src.type = ["square", "triangle", "sawtooth"][snd.wave] || "square";
      if (snd.wave === 0 && snd.duty != null && Math.abs(snd.duty - 0.5) > 0.01) src.setPeriodicWave(pulseWave(snd.duty));
      src.connect(g); freqParam = src.frequency;
    }
    const lvl = Math.max(0, Math.min(1, snd.vol)) * (snd.wave === 3 ? 0.5 : 0.16);
    const att = Math.max(0.003, snd.attack), rel = Math.max(0.01, snd.release);
    g.gain.setValueAtTime(0, t);
    if (snd.seq) {
      snd.seq.forEach((f, k) => {
        const tk = t + k * snd.step;
        if (f) freqParam.setValueAtTime(f, tk);
        g.gain.setValueAtTime(f ? lvl : 0, tk + (k === 0 ? 0 : 0));
      });
      g.gain.setValueAtTime(snd.seq[snd.seq.length - 1] ? lvl : 0, t + dur);
    } else {
      freqParam.setValueAtTime(Math.max(1, snd.from), t);
      freqParam.linearRampToValueAtTime(Math.max(1, snd.to), t + Math.max(0.001, dur));
      g.gain.linearRampToValueAtTime(lvl, t + Math.min(att, Math.max(0.003, dur)));
      g.gain.setValueAtTime(lvl, t + Math.max(att, dur));
    }
    g.gain.linearRampToValueAtTime(0, t + dur + rel);
    g.connect(master);
    src.start(t); src.stop(t + dur + rel + 0.05);
    vo.nodes = { src, g };
  } catch (e) { /* ignore */ }
}
