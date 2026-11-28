export const SILENCE_MS = 3000;
export const SILENCE_CHOICES = [2000, 3000, 4000];

export const joinResults = (parts, from = 0) =>
  parts.slice(from).map((part) => String(part ?? "").trim()).filter(Boolean).join(" ");

export const createQuietWatch = ({ ms = () => SILENCE_MS, now = Date.now, level = 0.04 }) => {
  let spoke = false;
  let lastAt = 0;
  return {
    sample: (value) => {
      if (value >= level) {
        spoke = true;
        lastAt = now();
      }
    },
    quiet: () => spoke && now() - lastAt >= ms(),
    reset: () => {
      spoke = false;
      lastAt = 0;
    },
  };
};
