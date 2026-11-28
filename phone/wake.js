export const WAKE_CALL_MS = 15000;

const words = (text) => String(text ?? "").toLowerCase().replace(/[^\p{L}\p{N}\s']/gu, " ").split(/\s+/).filter(Boolean);

export const hasName = (text, name = "nixa") => words(text).includes(String(name).toLowerCase());

export const createWake = ({ now = Date.now, name = () => "nixa" } = {}) => {
  let on = false;
  let calledUntil = 0;
  return {
    setOn: (value) => {
      on = value === true;
      if (!on) calledUntil = 0;
    },
    isOn: () => on,
    state: () => (!on ? "off" : now() < calledUntil ? "active" : "listening"),
    hear: (text) => {
      const line = String(text ?? "").trim();
      if (!on || !line) return "";
      if (hasName(line, name())) {
        calledUntil = now() + WAKE_CALL_MS;
        return line;
      }
      if (now() < calledUntil) {
        calledUntil = now() + WAKE_CALL_MS;
        return line;
      }
      return "";
    },
  };
};
