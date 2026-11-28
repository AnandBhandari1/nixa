import { ANIMATED, LOOKS, drawCore, paletteOf } from "./core.js";
import { kindLabel, kindOf } from "./chat-kinds.js";
import { createWake } from "./wake.js";
import { SILENCE_CHOICES, SILENCE_MS, createQuietWatch, joinResults } from "./silence.js";

const HERE_MS = 2000;
const DELAYS = [500, 1000, 2000, 4000];
const LAST_DELAY = 5000;
const TALK_HOLD_MS = 700;
const MIC_TEST_MS = 3000;
const MIC_RATE = 16000;
const PRESS_HOLD_MS = 350;
const TALK_MAX_MS = 180000;
const WAKE_PREROLL_MS = 3000;
const TALK_TAIL_MS = 250;
const TALK_MIN_MS = 300;
const SEND_SAMPLES = 16000;
const RECORD_TYPES = ["audio/mp4", "audio/webm;codecs=opus", "audio/webm"];
const NOTE_MS = 5000;
const DECODE_MS = 3000;
const CORE_MIN_PX = 150;
const SHORT_PX = 300;
const PAGE_FILES = ["/", "/app.js", "/core.js", "/chat-kinds.js", "/silence.js", "/wake.js"];
const RELOAD_WAIT_MS = 5000;
const UPDATED_KEY = "nixa.updated";
const SOUND_KEY = "nixa.sound";
const VIEW_KEY = "nixa.view";
const SILENCE_KEY = "nixa.silence_ms";
const CHAT_SHOWN = 40;
const PROBE_MS = 1000;
const BACK_WAIT_MS = 500;
const LINE_SLACK_MS = 1500;
const NAMES = {
  secure: "Page is https",
  connected: "Live connection to Nixa",
  live: "Nixa state arrives",
  rtt: "Round trip on Wi-Fi",
  mic: "Mic opens",
  sound: "Sound plays",
};

const $ = (id) => document.getElementById(id);

const readPref = (key) => {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
};

const writePref = (key, value) => {
  try {
    localStorage.setItem(key, value);
  } catch {}
};

const view = {
  audio: null,
  probing: false,
  soundWorked: false,
  soundLost: null,
  soundWanted: readPref(SOUND_KEY) !== "off",
  chatKey: "",
  talk: null,
  wakeTalk: null,
  note: null,
  build: null,
  buildCheck: null,
  reloading: false,
  current: 0,
  lines: new Map(),
  soundBase: "",
  socket: null,
  connected: false,
  tries: 0,
  hereTimer: null,
  snapshot: null,
  target: 0,
  level: 0,
  talkingAt: 0,
  shownMode: "",
  chatPinned: true,
  started: performance.now(),
  checks: {},
};

const send = (message) => {
  const socket = view.socket;
  if (!socket || socket.readyState !== WebSocket.OPEN) return false;
  socket.send(JSON.stringify(message));
  return true;
};

const renderChecks = () => {
  const list = $("list");
  list.replaceChildren(...Object.entries(NAMES).map(([name, label]) => {
    const check = view.checks[name];
    const li = document.createElement("li");
    li.className = check ? (check.ok ? "ok" : "bad") : "wait";
    const mark = document.createElement("span");
    mark.className = "mark";
    mark.textContent = check ? (check.ok ? "✓" : "✗") : "…";
    const body = document.createElement("div");
    const title = document.createElement("div");
    title.textContent = label;
    const detail = document.createElement("div");
    detail.className = "detail";
    detail.textContent = check?.detail ?? "not checked yet";
    body.append(title, detail);
    li.append(mark, body);
    return li;
  }));
};

const report = (name, ok, detail) => {
  view.checks[name] = { ok, detail, sent: false };
  renderChecks();
  if (view.connected) {
    send({ type: "check", name, ok, detail });
    view.checks[name].sent = true;
  }
};

const sendWaiting = () => {
  for (const [name, check] of Object.entries(view.checks)) {
    if (check.sent) continue;
    send({ type: "check", name, ok: check.ok, detail: check.detail });
    check.sent = true;
  }
};

const setStatus = (word, kind) => {
  const el = $("status");
  el.textContent = word;
  el.className = kind;
  $("dot").className = `dot ${kind}`;
  renderToggles();
};

const modeOf = () => {
  const snap = view.snapshot;
  if (!view.connected || !snap) return "wait";
  if (view.talk && !view.talk.ending) return "you";
  if (Date.now() - view.talkingAt < TALK_HOLD_MS) return "you";
  if (snap.state === "speaking") return !snap.mobile ? "nixa" : !view.soundWanted ? "muted" : voiceRunning() ? "nixa" : "silent";
  if (snap.state === "thinking") return "think";
  if (snap.state === "paused") return "paused";
  if (snap.state === "off") return "off";
  if (snap.mobile) return "ready";
  if (snap.micOff) return "micoff";
  return snap.hearing ? "listen" : "standby";
};

const voiceRunning = () => view.audio?.state === "running";

const renderToggles = () => {
  const running = voiceRunning();
  const sound = $("soundbtn");
  sound.classList.toggle("on", view.soundWanted && running);
  sound.classList.toggle("warn", view.soundWanted && !running);
  $("soundword").textContent = !view.soundWanted ? "Muted" : running ? "Sound" : "Tap";
  $("soundwave").style.display = view.soundWanted ? "" : "none";
  const mobile = view.snapshot?.mobile === true;
  const place = $("placebtn");
  place.classList.toggle("on", mobile);
  $("placeword").textContent = mobile ? "Phone" : "Mac";
  const note = view.note && Date.now() < view.note.until ? view.note.text : "";
  const hint = note ? note : !view.connected && view.tries > 0
    ? "Reconnecting to Nixa on the Mac…"
    : view.soundWanted && !running ? "Tap anywhere to turn on sound." : !mobile && view.snapshot ? "Nixa speaks on the Mac. Tap Mac to switch to this phone." : "";
  $("hint").textContent = hint;
  $("hint").hidden = !hint;
};

const showNote = (text) => {
  view.note = { text, until: Date.now() + NOTE_MS };
  renderToggles();
  setTimeout(renderToggles, NOTE_MS + 50);
};

const tellSound = () => {
  if (voiceRunning()) view.soundWorked = true;
  if (voiceRunning() && view.soundLost) {
    const was = view.soundLost;
    view.soundLost = null;
    report("sound", true, `sound back after a tap (it stopped: ${was})`);
  }
  send({ type: view.soundWanted && voiceRunning() ? "voice_ready" : "voice_off" });
  renderToggles();
};

const clockOf = (at) => {
  const date = new Date(Number(at) || Date.now());
  return `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
};

const chatRow = (item) => {
  const row = document.createElement("div");
  const kind = item.who === "user" ? "user" : item.who === "nixa" ? "nixa" : item.who === "agent" ? (item.to ? "to" : "agent") : "info";
  row.className = `msg ${kind}`;
  row.textContent = String(item.text ?? "");
  const tag = document.createElement("span");
  tag.className = "kind";
  tag.textContent = kindLabel(kindOf(item));
  row.append(tag);
  if (kind !== "info") {
    const meta = document.createElement("span");
    meta.className = "meta";
    const who = kind === "user" ? (item.typed ? "You typed" : "You said") : kind === "nixa" ? "Nixa" : String(item.label ?? (kind === "to" ? "Nixa to agent" : "Agent"));
    meta.textContent = `${who} · ${clockOf(item.at)}`;
    row.append(meta);
  }
  return row;
};

const renderChat = (chat) => {
  const items = (Array.isArray(chat) ? chat : []).filter((item) => item && !item.quiet && typeof item.text === "string").slice(-CHAT_SHOWN);
  const key = items.map((item) => `${item.id}:${item.text.length}`).join(",");
  if (key === view.chatKey) return;
  const first = view.chatKey === "";
  view.chatKey = key;
  const box = $("chat");
  const atBottom = box.scrollHeight - box.scrollTop - box.clientHeight < 60;
  if (!items.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No talk yet. Hold the green button and speak.";
    box.replaceChildren(empty);
    return;
  }
  box.replaceChildren(...items.map(chatRow));
  if (atBottom || first) box.scrollTop = box.scrollHeight;
};

const startSound = () => {
  if (view.audio) return;
  try {
    view.audio = new AudioContext();
    view.audio.onstatechange = tellSound;
    view.audio.resume().catch(() => null).then(tellSound);
  } catch {}
};

const unlockAudio = () => {
  try {
    if (navigator.audioSession) navigator.audioSession.type = "playback";
  } catch {}
  if (!view.audio) {
    view.audio = new AudioContext();
    view.audio.onstatechange = tellSound;
  }
  const blank = view.audio.createBuffer(1, 1, 22050);
  const source = view.audio.createBufferSource();
  source.buffer = blank;
  source.connect(view.audio.destination);
  source.start(0);
  view.audio.resume().catch(() => null).then(tellSound);
};

const stopLine = (line) => {
  const rec = view.lines.get(line);
  if (!rec) return;
  view.lines.delete(line);
  for (const source of rec.sources) {
    source.onended = null;
    try {
      source.stop();
    } catch {}
  }
};

const stopAll = () => {
  for (const line of [...view.lines.keys()]) stopLine(line);
};

const lineDone = (line) => {
  const rec = view.lines.get(line);
  if (!rec || rec.parts === null || rec.ended < rec.parts) return;
  view.lines.delete(line);
  send({ type: "played", line });
};

const dropSound = (why) => {
  const old = view.audio;
  if (!old) return;
  view.audio = null;
  old.onstatechange = null;
  old.close().catch(() => null);
  stopAll();
  const worked = view.soundWorked;
  view.soundLost = why;
  tellSound();
  report("sound", false, `sound stopped (${why}), waits for a tap`);
  if (worked) showNote("Sound stopped on this phone. Tap anywhere to turn it back on.");
};

const probeSound = (ctx) => new Promise((resolve) => {
  try {
    const source = ctx.createBufferSource();
    source.buffer = ctx.createBuffer(1, Math.max(1, Math.round(ctx.sampleRate * 0.05)), ctx.sampleRate);
    source.connect(ctx.destination);
    const timer = setTimeout(() => {
      source.onended = null;
      resolve(false);
    }, PROBE_MS);
    source.onended = () => {
      clearTimeout(timer);
      resolve(true);
    };
    source.start();
  } catch {
    resolve(false);
  }
});

const checkSound = async (why) => {
  const ctx = view.audio;
  if (!ctx || !view.soundWanted || view.probing || document.visibilityState !== "visible") return;
  view.probing = true;
  await ctx.resume().catch(() => null);
  const alive = ctx.state === "running" && await probeSound(ctx);
  view.probing = false;
  if (view.audio !== ctx || document.visibilityState !== "visible") return;
  if (alive || !view.soundWorked) tellSound();
  else dropSound(why);
};

const watchLine = (line, rec) => {
  const ctx = view.audio;
  if (!ctx) return;
  const waitMs = Math.max(0, rec.nextTime - ctx.currentTime) * 1000 + LINE_SLACK_MS;
  setTimeout(() => {
    if (view.lines.get(line) !== rec) return;
    stopLine(line);
    if (rec.nextTime > 0 && view.audio === ctx && document.visibilityState === "visible") dropSound("a line did not finish");
  }, waitMs);
};

const onAudio = (data) => {
  const ctx = view.audio;
  if (!ctx || data.byteLength < 10) return;
  const head = new DataView(data, 0, 8);
  const line = head.getUint32(0, true);
  const rec = view.lines.get(line);
  if (!rec || line !== view.current) return;
  const pcm = new Int16Array(data, 8, Math.floor((data.byteLength - 8) / 2));
  const samples = new Float32Array(pcm.length);
  for (let i = 0; i < pcm.length; i++) samples[i] = pcm[i] / 32768;
  const buffer = ctx.createBuffer(1, samples.length, rec.rate);
  buffer.copyToChannel(samples, 0);
  const source = ctx.createBufferSource();
  source.buffer = buffer;
  source.connect(ctx.destination);
  const at = Math.max(ctx.currentTime + 0.05, rec.nextTime);
  source.start(at);
  rec.nextTime = at + buffer.duration;
  rec.sources.add(source);
  source.onended = () => {
    rec.sources.delete(source);
    rec.ended += 1;
    lineDone(line);
  };
};

const makeResampler = (rate) => {
  const step = rate / MIC_RATE;
  let sum = 0;
  let count = 0;
  let at = 0;
  let next = step;
  return (input) => {
    const out = [];
    for (let i = 0; i < input.length; i++) {
      sum += input[i];
      count += 1;
      at += 1;
      if (at >= next) {
        out.push(sum / count);
        sum = 0;
        count = 0;
        next += step;
      }
    }
    const pcm = new Int16Array(out.length);
    for (let i = 0; i < out.length; i++) pcm[i] = Math.max(-32768, Math.min(32767, Math.round(out[i] * 32767)));
    return pcm;
  };
};

const renderTalk = () => {
  const button = $("talkbtn");
  const talk = view.talk;
  button.classList.toggle("live", Boolean(talk));
  $("talkword").textContent = !talk ? "Talk" : talk.ending ? "Sending" : !talk.stream ? "Opening" : talk.held ? "Listening" : "Tap to send";
  button.setAttribute("aria-label", !talk ? "Hold or tap to talk" : talk.ending ? "Sending" : !talk.stream ? "Opening the mic" : talk.held ? "Listening, let go to send" : "Listening, tap to send");
};

const finishTalk = (talk) => {
  clearTimeout(talk.timer);
  talk.node?.disconnect();
  talk.source?.disconnect();
  try {
    if (talk.recorder && talk.recorder.state !== "inactive") talk.recorder.stop();
  } catch {}
  talk.stream?.getTracks().forEach((track) => track.stop());
  try {
    if (navigator.audioSession) navigator.audioSession.type = "playback";
  } catch {}
  view.target = 0;
};

const errorText = (error) => `${error?.name ?? "Error"} ${error?.message ?? error ?? ""}`.trim().slice(0, 60);

const errorName = (error) => (error?.name && error.name !== "Error" ? error.name : String(error?.message ?? error ?? "unknown")).slice(0, 30);

const withTimeout = (promise, ms) => new Promise((resolve, reject) => {
  const timer = setTimeout(() => reject(new Error("timed out")), ms);
  Promise.resolve(promise).then((value) => {
    clearTimeout(timer);
    resolve(value);
  }, (error) => {
    clearTimeout(timer);
    reject(error);
  });
});

const startRecorder = (talk) => {
  if (typeof MediaRecorder === "undefined") {
    talk.recordNote = "no MediaRecorder";
    return;
  }
  try {
    const type = RECORD_TYPES.find((kind) => MediaRecorder.isTypeSupported?.(kind));
    const recorder = new MediaRecorder(talk.stream, type ? { mimeType: type } : undefined);
    recorder.ondataavailable = (event) => {
      if (event.data?.size) talk.recorded.push(event.data);
    };
    recorder.addEventListener("error", (event) => {
      talk.recordError = errorName(event.error ?? event);
    });
    recorder.addEventListener("stop", () => {
      talk.recordStopped = true;
    });
    recorder.start();
    talk.recorder = recorder;
  } catch (error) {
    talk.recordNote = `start failed: ${errorText(error)}`;
  }
};

const stopRecorder = (recorder) => new Promise((resolve) => {
  if (!recorder || recorder.state === "inactive") {
    resolve();
    return;
  }
  const timer = setTimeout(resolve, 1500);
  recorder.addEventListener("stop", () => {
    clearTimeout(timer);
    resolve();
  }, { once: true });
  try {
    recorder.stop();
  } catch {
    clearTimeout(timer);
    resolve();
  }
});

const readRecording = async (talk) => {
  const empty = new Int16Array(0);
  if (!talk.recorder) return { pcm: empty, note: talk.recordNote ?? "not started" };
  const type = (talk.recorder.mimeType || "no type").replace("audio/", "").replace("codecs=", "");
  if (!talk.recorded.length) return { pcm: empty, note: `${type}, no data${talk.recordError ? `, error ${talk.recordError}` : ""}${talk.recordStopped ? "" : ", did not stop"}` };
  const blob = new Blob(talk.recorded, { type: talk.recorder.mimeType || "" });
  const head = `${type}, ${Math.round(blob.size / 1024)} KB`;
  let readError = null;
  const data = await blob.arrayBuffer().catch((error) => {
    readError = error;
    return null;
  });
  if (!data) return { pcm: empty, note: `${head}, read failed: ${errorName(readError)}` };
  const decoders = [];
  const failed = [];
  if (typeof OfflineAudioContext !== "function") failed.push("offline none");
  else {
    try {
      decoders.push(["offline", new OfflineAudioContext(1, 1, MIC_RATE)]);
    } catch (error) {
      failed.push(`offline ${errorName(error)}`);
    }
  }
  if (view.audio) decoders.push(["page", view.audio]);
  else failed.push("page none");
  for (const [name, ctx] of decoders) {
    try {
      const buffer = await ctx.decodeAudioData(data.slice(0));
      if (!buffer?.length) throw new Error("empty");
      return { pcm: makeResampler(buffer.sampleRate)(buffer.getChannelData(0)), note: `${head}, ${name} decode ${buffer.sampleRate} Hz` };
    } catch (error) {
      failed.push(`${name} ${errorName(error)}`);
    }
  }
  return { pcm: empty, note: `${head}, decode failed (${failed.join(", ")})` };
};

const joinPcm = (parts) => {
  const out = new Int16Array(parts.reduce((total, part) => total + part.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
};

const sendPcm = (pcm) => {
  const socket = view.socket;
  if (!socket || socket.readyState !== WebSocket.OPEN) return;
  for (let at = 0; at < pcm.length; at += SEND_SAMPLES) socket.send(pcm.slice(at, at + SEND_SAMPLES).buffer);
};

const seconds = (samples) => (samples / MIC_RATE).toFixed(1);

const browserTag = () => {
  const ua = navigator.userAgent;
  const ios = ua.match(/OS (\d+)_(\d+)/);
  if (/iPhone|iPad/.test(ua) && ios) return `iOS ${ios[1]}.${ios[2]}`;
  const chrome = ua.match(/Chrome\/(\d+)/);
  if (chrome) return `Chrome ${chrome[1]}`;
  const safari = ua.match(/Version\/(\d+\.\d+).*Safari/);
  return safari ? `Safari ${safari[1]}` : "other browser";
};

const micState = (talk) => {
  const track = talk.stream?.getAudioTracks()[0];
  if (!track) return "mic none";
  const state = track.readyState !== "live" ? track.readyState : track.muted ? "muted" : "live";
  return `mic ${state}${talk.mutedSeen && state !== "muted" ? " (was muted)" : ""}`;
};

const soundState = (talk) => {
  const now = view.audio?.state ?? "none";
  const then = talk.soundAtMic ?? now;
  return `sound ${then}${then !== now ? `→${now}` : ""}`;
};

const sendTalk = async (talk) => {
  if (view.talk !== talk || talk.cancelled) return;
  const live = joinPcm(talk.chunks);
  const mic = micState(talk);
  await stopRecorder(talk.recorder);
  if (view.talk !== talk || talk.cancelled) return;
  finishTalk(talk);
  const read = await withTimeout(readRecording(talk), DECODE_MS).catch((error) => ({ pcm: new Int16Array(0), note: error?.message === "timed out" ? `no answer in ${DECODE_MS / 1000} s` : `read ${errorText(error)}` }));
  if (view.talk !== talk || talk.cancelled) return;
  const recorded = read.pcm;
  const useRecording = recorded.length - live.length > MIC_RATE / 2;
  const pcm = (useRecording ? recorded : live).subarray(0, (MIC_RATE * TALK_MAX_MS) / 1000);
  sendPcm(pcm);
  send({ type: "talk_end" });
  if (view.talk === talk) view.talk = null;
  renderTalk();
  startWake();
  const enough = pcm.length >= (MIC_RATE * TALK_MIN_MS) / 1000;
  report("mic", enough, `talk ${seconds(pcm.length)} s, live mic ${seconds(live.length)} s, recording ${seconds(recorded.length)} s (${read.note})${useRecording ? ", used recording" : ""}, ${mic}, ${soundState(talk)}, ${browserTag()}`);
  if (!enough) showNote(talk.stream ? "Nixa got no sound. Hold the button while you speak." : "The mic did not open. Allow the mic, then try again.");
  if (talk.soundAtMic === "running" && live.length === 0 && recorded.length > MIC_RATE / 2) {
    dropSound("the live mic got nothing");
    return;
  }
  view.audio?.resume().catch(() => null).then(tellSound);
};

const endTalk = (cancel) => {
  const talk = view.talk;
  if (!talk || (talk.ending && !cancel)) return;
  talk.ending = true;
  renderTalk();
  if (!cancel) {
    talk.sendTimer = setTimeout(() => void sendTalk(talk), TALK_TAIL_MS);
    return;
  }
  talk.cancelled = true;
  clearTimeout(talk.sendTimer);
  if (view.talk === talk) view.talk = null;
  finishTalk(talk);
  send({ type: "talk_cancel" });
  renderTalk();
  startWake();
  view.audio?.resume().catch(() => null).then(tellSound);
};

const newTalkCapture = () => ({
  pressAt: 0, held: false, woken: false, ending: false, cancelled: false, opening: null, sendTimer: null,
  stream: null, source: null, node: null, recorder: null, recordNote: null,
  chunks: [], samples: 0, recorded: [], timer: null, mutedSeen: false,
  soundAtMic: null, recordError: null, recordStopped: false,
});

const openTalkCapture = async (talk) => {
  try {
    if (!navigator.mediaDevices?.getUserMedia) throw new Error("No mic access: use a secure HTTPS page.");
    if (!view.audio) unlockAudio();
    try {
      if (navigator.audioSession) navigator.audioSession.type = "play-and-record";
    } catch {}
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 } });
    if ((view.talk !== talk && view.wakeTalk !== talk) || talk.ending) {
      stream.getTracks().forEach((track) => track.stop());
      return;
    }
    talk.stream = stream;
    stream.getAudioTracks()[0]?.addEventListener("mute", () => {
      talk.mutedSeen = true;
    });
    const ctx = view.audio;
    if (ctx.state !== "running") showNote("Tap the screen once to enable microphone capture.");
    await ctx.resume().catch(() => null);
    if ((view.talk !== talk && view.wakeTalk !== talk) || talk.ending) return;
    talk.soundAtMic = ctx.state;
    const source = ctx.createMediaStreamSource(stream);
    const node = ctx.createScriptProcessor(4096, 1, 1);
    const resample = makeResampler(ctx.sampleRate);
    node.onaudioprocess = (event) => {
      const active = view.talk === talk;
      if (!active && view.wakeTalk !== talk) return;
      if (!active && view.snapshot?.state === "speaking") return;
      const input = event.inputBuffer.getChannelData(0);
      const pcm = resample(input);
      if (pcm.length) {
        talk.chunks.push(pcm);
        talk.samples += pcm.length;
      }
      if (!active) {
        while (talk.samples > (MIC_RATE * WAKE_PREROLL_MS) / 1000 && talk.chunks.length > 1) {
          talk.samples -= talk.chunks.shift().length;
        }
        return;
      }
      let sum = 0;
      for (const v of input) sum += v * v;
      view.target = Math.min(1, Math.sqrt(sum / input.length) * 6);
      view.talkingAt = Date.now();
      quiet.sample(view.target);
      if (!talk.held && !talk.ending && quiet.quiet()) endTalk(false);
    };
    source.connect(node);
    node.connect(ctx.destination);
    Object.assign(talk, { source, node });
  } catch (error) {
    if (view.talk !== talk && view.wakeTalk !== talk) return;
    wake.setOn(false);
    stopWake();
    if (view.talk === talk) endTalk(true);
    report("mic", false, `${error?.name ?? "Error"}: ${error?.message ?? error}`);
    renderWake();
  }
};

const startTalk = async (woken = false) => {
  if (view.talk) return;
  stopAll();
  const talk = view.wakeTalk ?? newTalkCapture();
  view.wakeTalk = null;
  view.talk = talk;
  talk.pressAt = performance.now();
  talk.held = !woken;
  talk.woken = woken;
  stopWake();
  talk.timer = setTimeout(() => endTalk(false), TALK_MAX_MS);
  quiet.reset();
  if (woken) quiet.sample(1);
  send({ type: "talk_start" });
  renderTalk();
  renderWake();
  talk.opening ??= openTalkCapture(talk);
  await talk.opening;
  if (view.talk !== talk || talk.ending || !talk.stream) return;
  view.note = null;
  startRecorder(talk);
  renderToggles();
  renderTalk();
};

const releaseTalk = () => {
  const talk = view.talk;
  if (!talk || !talk.held || talk.ending) return;
  const long = performance.now() - talk.pressAt >= PRESS_HOLD_MS;
  if (long && talk.stream) {
    endTalk(false);
    return;
  }
  talk.held = false;
  renderTalk();
  if (long) showNote("The mic is still opening. Speak, then tap the button to send.");
};

const onMessage = (message) => {
  if (message?.type === "audio_start") {
    stopAll();
    view.current = Number(message.line);
    view.lines.set(view.current, { rate: Number(message.rate) || 24000, sources: new Set(), parts: null, ended: 0, nextTime: 0 });
    return;
  }
  if (message?.type === "audio_end") {
    const rec = view.lines.get(Number(message.line));
    if (rec) rec.parts = Number(message.parts);
    lineDone(Number(message.line));
    if (rec && view.lines.get(Number(message.line)) === rec) watchLine(Number(message.line), rec);
    return;
  }
  if (message?.type === "audio_stop") {
    stopLine(Number(message.line));
    return;
  }
  if (message?.type === "here") {
    const rtt = Math.round(performance.now() - Number(message.t));
    if (!view.checks.rtt) report("rtt", rtt < 1000, `${rtt} ms`);
    return;
  }
  if (message?.type === "snapshot") {
    view.snapshot = message.snapshot ?? null;
    if (view.snapshot?.state === "speaking" && view.wakeTalk) {
      view.wakeTalk.chunks = [];
      view.wakeTalk.samples = 0;
    }
    if (view.snapshot?.userTalking) view.talkingAt = Date.now();
    if (!view.checks.live) report("live", true, `Nixa is ${view.snapshot?.state ?? "unknown"}`);
    renderChat(view.snapshot?.chat);
    if (!wakeChosen && view.snapshot?.listen === "wake" && Recognition) {
      wakeChosen = true;
      wake.setOn(true);
      startWake();
    }
    renderToggles();
    return;
  }
  if (message?.type === "level") {
    view.target = Math.max(0, Math.min(1, Number(message.mic) || 0));
    if (message.talking) view.talkingAt = Date.now();
    return;
  }
};

const hashText = (text) => {
  let hash = 2166136261;
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(16);
};

const pageBuild = async () => {
  try {
    const texts = await Promise.all(PAGE_FILES.map(async (path) => {
      const res = await fetch(path, { cache: "no-store" });
      if (!res.ok) throw new Error(`${path} ${res.status}`);
      return res.text();
    }));
    return texts.map(hashText).join(".");
  } catch {
    return null;
  }
};

const pageBusy = () => Boolean(view.talk) || wake.state() === "active" || view.lines.size > 0 || $("typein").value.trim() !== "" || document.visibilityState !== "visible";

const reloadWhenFree = () => {
  if (pageBusy()) {
    setTimeout(reloadWhenFree, RELOAD_WAIT_MS);
    return;
  }
  try {
    sessionStorage.setItem(UPDATED_KEY, "1");
  } catch {}
  location.reload();
};

const checkBuild = () => {
  if (view.buildCheck || view.reloading) return view.buildCheck;
  view.buildCheck = pageBuild().then((build) => {
    view.buildCheck = null;
    if (!build) return;
    if (!view.build) {
      view.build = build;
      return;
    }
    if (build === view.build || view.reloading) return;
    view.reloading = true;
    reloadWhenFree();
  });
  return view.buildCheck;
};

const connect = () => {
  const socket = new WebSocket(`wss://${location.host}/ws`);
  socket.binaryType = "arraybuffer";
  view.socket = socket;
  setStatus(view.tries ? "reconnecting" : "connecting", "");
  socket.onopen = () => {
    view.connected = true;
    view.tries = 0;
    setStatus("connected", "on");
    if (!view.checks.connected?.ok) report("connected", true, "WebSocket open");
    sendWaiting();
    send({ type: "here", t: performance.now() });
    if (view.soundWanted && voiceRunning()) send({ type: "voice_ready" });
    startWake();
    renderToggles();
    view.hereTimer = setInterval(() => send({ type: "here", t: performance.now() }), HERE_MS);
    void checkBuild();
  };
  socket.onmessage = (event) => {
    if (event.data instanceof ArrayBuffer) {
      onAudio(event.data);
      return;
    }
    try {
      onMessage(JSON.parse(event.data));
    } catch {}
  };
  socket.onclose = (event) => {
    clearInterval(view.hereTimer);
    view.hereTimer = null;
    const was = view.connected;
    view.connected = false;
    view.socket = null;
    stopAll();
    stopWake();
    if (view.talk) endTalk(true);
    if (!was && !view.checks.connected?.ok) report("connected", false, `could not connect (code ${event.code})`);
    if (event.code === 4001) {
      setStatus("removed on the Mac", "off");
      return;
    }
    setStatus("reconnecting", "off");
    const delay = view.tries < DELAYS.length ? DELAYS[view.tries] : LAST_DELAY;
    view.tries += 1;
    setTimeout(connect, delay);
  };
};

const testMic = async () => {
  if (!navigator.mediaDevices?.getUserMedia) {
    report("mic", false, "no mic access: the page is not a secure page");
    return;
  }
  let stream = null;
  let ctx = null;
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    ctx = new AudioContext();
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 1024;
    ctx.createMediaStreamSource(stream).connect(analyser);
    const data = new Float32Array(analyser.fftSize);
    let loudest = 0;
    report("mic", true, "open, say something for 3 seconds");
    const until = performance.now() + MIC_TEST_MS;
    await new Promise((resolve) => {
      const tick = () => {
        analyser.getFloatTimeDomainData(data);
        let sum = 0;
        for (const v of data) sum += v * v;
        const rms = Math.sqrt(sum / data.length);
        loudest = Math.max(loudest, rms);
        view.target = Math.min(1, rms * 6);
        view.talkingAt = rms > 0.02 ? Date.now() : view.talkingAt;
        if (performance.now() < until) requestAnimationFrame(tick);
        else resolve();
      };
      tick();
    });
    const rate = stream.getAudioTracks()[0]?.getSettings?.().sampleRate;
    report("mic", true, `opened${rate ? ` at ${rate} Hz` : ""}, loudest ${Math.round(Math.min(1, loudest * 6) * 100)}%`);
  } catch (error) {
    report("mic", false, `${error?.name ?? "Error"}: ${error?.message ?? error}`);
  } finally {
    stream?.getTracks().forEach((track) => track.stop());
    await ctx?.close().catch(() => null);
    view.target = 0;
  }
};

const testSound = async () => {
  try {
    const ctx = new AudioContext();
    await ctx.resume();
    const tone = ctx.createOscillator();
    const gain = ctx.createGain();
    tone.frequency.value = 660;
    gain.gain.value = 0.2;
    tone.connect(gain).connect(ctx.destination);
    tone.start();
    tone.stop(ctx.currentTime + 0.35);
    const state = ctx.state;
    const rate = ctx.sampleRate;
    setTimeout(() => ctx.close().catch(() => null), 800);
    view.soundBase = `audio ${state} at ${rate} Hz`;
    report("sound", state === "running", `${view.soundBase}, did you hear it?`);
    $("ask").hidden = state !== "running";
  } catch (error) {
    report("sound", false, `${error?.name ?? "Error"}: ${error?.message ?? error}`);
  }
};

const heard = (yes) => {
  $("ask").hidden = true;
  report("sound", yes, `${view.soundBase}, beep ${yes ? "heard" : "not heard"}`);
};

const showFull = (full) => {
  $("app").classList.toggle("full", full);
  $("viewbtn").classList.toggle("on", full);
  writePref(VIEW_KEY, full ? "full" : "chat");
  if (!full) {
    const box = $("chat");
    box.scrollTop = box.scrollHeight;
  }
};

const showMode = (mode) => {
  if (mode === view.shownMode) return;
  view.shownMode = mode;
  const look = LOOKS[mode] ?? LOOKS.standby;
  const button = $("modebtn");
  button.style.setProperty("--tone", paletteOf(mode).main);
  button.classList.toggle("live", ANIMATED.has(mode));
  $("modeword").textContent = look.word;
  $("modesub").textContent = look.sub;
};

const frame = () => {
  const mode = modeOf();
  showMode(mode);
  const canvas = $("core");
  const box = $("corebox").getBoundingClientRect();
  const size = Math.floor(Math.min(box.width, box.height));
  canvas.hidden = size < CORE_MIN_PX;
  if (canvas.hidden) {
    requestAnimationFrame(frame);
    return;
  }
  const ratio = window.devicePixelRatio || 1;
  if (canvas.width !== Math.round(size * ratio)) {
    canvas.width = Math.round(size * ratio);
    canvas.height = Math.round(size * ratio);
    canvas.style.width = `${size}px`;
    canvas.style.height = `${size}px`;
  }
  const ctx = canvas.getContext("2d");
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  view.level += (view.target - view.level) * 0.3;
  const look = LOOKS[mode] ?? LOOKS.standby;
  drawCore(ctx, {
    size,
    t: (performance.now() - view.started) / 1000,
    mode,
    level: mode === "you" ? view.level : mode === "nixa" ? 0.6 : 0,
    word: look.word,
    sub: look.sub,
  });
  requestAnimationFrame(frame);
};

for (const kind of ["touchend", "click"]) {
  document.addEventListener(kind, () => {
    if ((view.soundWanted || wake.isOn()) && !voiceRunning()) unlockAudio();
  });
}
$("soundbtn").addEventListener("click", () => {
  view.soundWanted = !view.soundWanted;
  writePref(SOUND_KEY, view.soundWanted ? "on" : "off");
  if (view.soundWanted) unlockAudio();
  else {
    stopAll();
    tellSound();
  }
});

let silenceMs = SILENCE_MS;
const savedSilence = Number(readPref(SILENCE_KEY));
if (SILENCE_CHOICES.includes(savedSilence)) silenceMs = savedSilence;
const quiet = createQuietWatch({ ms: () => silenceMs });
const wake = createWake({ name: () => view.snapshot?.name || "nixa" });
const Recognition = window.SpeechRecognition || window.webkitSpeechRecognition;
let recognizer = null;
let wakeChosen = false;
const renderSilence = () => {
  $("silencebtn").textContent = `Auto-send after ${silenceMs / 1000} seconds of silence`;
};
const renderWake = () => {
  const btn = $("wakebtn");
  const state = wake.isOn() && view.talk?.woken ? "active" : wake.state();
  btn.classList.toggle("on", state !== "off");
  btn.setAttribute("aria-pressed", String(state !== "off"));
  btn.textContent = `Wake word: ${state === "off" ? "off" : "on"}`;
  $("wakestate").textContent = !Recognition ? "This browser cannot hear the word Nixa." : state === "off" ? "" : view.wakeTalk && !recognizer ? "Opening microphone for wake word…" : state === "active" ? "Active: I am listening to you." : "Listening for Nixa.";
};
const stopWake = () => {
  const r = recognizer;
  recognizer = null;
  const standby = view.wakeTalk;
  view.wakeTalk = null;
  if (standby) {
    standby.ending = true;
    finishTalk(standby);
  }
  try { r?.stop(); } catch {}
};
const startWake = async () => {
  const ready = () => Recognition && wake.isOn() && !recognizer && view.connected && !view.talk && document.visibilityState === "visible";
  if (!ready()) return;
  const standby = view.wakeTalk ?? newTalkCapture();
  view.wakeTalk = standby;
  standby.opening ??= openTalkCapture(standby);
  await standby.opening;
  if (!ready() || view.wakeTalk !== standby || !standby.node) return;
  const r = new Recognition();
  recognizer = r;
  r.continuous = true;
  r.interimResults = true;
  r.lang = navigator.language || "en-US";
  r.onresult = (event) => {
    if (recognizer !== r) return;
    const parts = Array.from(event.results, (result) => result[0].transcript);
    if (view.snapshot?.state === "speaking") return;
    const line = joinResults(parts);
    if (wake.hear(line)) void startTalk(true);
    renderWake();
  };
  r.onerror = (event) => {
    if (recognizer !== r) return;
    if (event.error === "not-allowed" || event.error === "service-not-allowed") {
      wake.setOn(false);
      stopWake();
      showNote("Allow the microphone to use the wake word.");
      renderWake();
    }
  };
  r.onend = () => {
    if (recognizer !== r) return;
    recognizer = null;
    if (wake.isOn()) setTimeout(startWake, 400);
    renderWake();
    renderToggles();
  };
  try {
    r.start();
  } catch (error) {
    wake.setOn(false);
    stopWake();
    showNote(`Wake word could not start: ${errorText(error)}`);
    renderWake();
  }
};
const setWake = (on) => {
  wakeChosen = true;
  wake.setOn(on && Boolean(Recognition));
  if (wake.isOn()) startWake();
  else {
    stopWake();
    if (view.talk?.woken) endTalk(true);
  }
  renderWake();
};
$("wakebtn").addEventListener("click", () => setWake(!wake.isOn()));
$("silencebtn").addEventListener("click", () => {
  silenceMs = SILENCE_CHOICES[(SILENCE_CHOICES.indexOf(silenceMs) + 1) % SILENCE_CHOICES.length];
  writePref(SILENCE_KEY, String(silenceMs));
  renderSilence();
});
renderSilence();
renderWake();
setInterval(renderWake, 1000);

$("placebtn").addEventListener("click", () => {
  send({ type: "set_mobile", on: view.snapshot?.mobile !== true });
});
$("viewbtn").addEventListener("click", () => showFull(!$("app").classList.contains("full")));
$("modebtn").addEventListener("click", () => showFull(true));
$("chat").addEventListener("scroll", () => {
  const box = $("chat");
  if (box.clientHeight > 0) view.chatPinned = box.scrollHeight - box.scrollTop - box.clientHeight < 60;
}, { passive: true });
new ResizeObserver(() => {
  const box = $("chat");
  if (view.chatPinned) box.scrollTop = box.scrollHeight;
}).observe($("chat"));
const fitShort = () => {
  const height = window.visualViewport?.height ?? window.innerHeight;
  $("app").classList.toggle("short", height <= SHORT_PX);
};
window.visualViewport?.addEventListener("resize", fitShort);
window.addEventListener("resize", fitShort);
$("infobtn").addEventListener("click", () => {
  $("info").hidden = false;
});
$("infoclose").addEventListener("click", () => {
  $("info").hidden = true;
});
$("info").addEventListener("click", (event) => {
  if (event.target === $("info")) $("info").hidden = true;
});
$("keybtn").addEventListener("click", () => {
  const row = $("typerow");
  row.hidden = !row.hidden;
  $("keybtn").classList.toggle("on", !row.hidden);
  if (!row.hidden) $("typein").focus();
});
$("typerow").addEventListener("submit", (event) => {
  event.preventDefault();
  const input = $("typein");
  const text = input.value.trim();
  if (!text) return;
  if (send({ type: "text", text })) input.value = "";
});
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") {
    stopAll();
    stopWake();
    if (view.talk) endTalk(true);
    send({ type: "voice_off" });
    return;
  }
  void checkBuild();
  startWake();
  if (!view.audio) return;
  setTimeout(() => void checkSound("the page came back"), BACK_WAIT_MS);
});
window.addEventListener("pageshow", (event) => {
  if (!event.persisted) return;
  void checkBuild();
  startWake();
  setTimeout(() => void checkSound("the page came back"), BACK_WAIT_MS);
});
$("talkbtn").addEventListener("pointerdown", (event) => {
  event.preventDefault();
  if (view.talk) {
    if (!view.talk.held) endTalk(false);
    return;
  }
  void startTalk();
});
$("talkbtn").addEventListener("pointerup", releaseTalk);
$("talkbtn").addEventListener("pointercancel", () => endTalk(true));
$("talkbtn").addEventListener("contextmenu", (event) => event.preventDefault());
$("mic").addEventListener("click", () => void testMic());
$("sound").addEventListener("click", () => void testSound());
$("yes").addEventListener("click", () => heard(true));
$("no").addEventListener("click", () => heard(false));
renderChecks();
renderToggles();
renderTalk();
fitShort();
showFull(readPref(VIEW_KEY) === "full");
if (view.soundWanted) startSound();
report("secure", window.isSecureContext === true, `${location.protocol}//${location.host}, secure page: ${window.isSecureContext ? "yes" : "no"}`);
try {
  if (sessionStorage.getItem(UPDATED_KEY)) {
    sessionStorage.removeItem(UPDATED_KEY);
    showNote("Nixa page updated.");
  }
} catch {}
void checkBuild();
connect();
requestAnimationFrame(frame);
