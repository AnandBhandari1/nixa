import os
import select
import sys
import time
import tty

LOG = os.environ.get("FAKE_LOG", "")
FD = sys.stdin.fileno()


def out(text):
    sys.stdout.write(text)
    sys.stdout.flush()


def status(state, note=""):
    tail = ";note=" + note if note else ""
    out("\x1b]7421;status=" + state + tail + "\x07")


def log(line):
    if LOG:
        with open(LOG, "a") as f:
            f.write(line + "\n")


def key():
    c = os.read(FD, 1)
    if c == b"\x1b":
        ready, _, _ = select.select([FD], [], [], 0.05)
        if not ready:
            return "esc"
        rest = os.read(FD, 2)
        return {b"[A": "up", b"OA": "up", b"[B": "down", b"OB": "down"}.get(rest, "esc")
    if c in (b"\r", b"\n"):
        return "enter"
    if c == b"\t":
        return "tab"
    return c.decode(errors="ignore")


def draw(lines):
    out("\x1b[2J\x1b[H" + "\r\n".join(lines) + "\r\n")


def line(head):
    draw(head)
    buf = ""
    while True:
        c = os.read(FD, 1)
        if c in (b"\r", b"\n"):
            if buf:
                return buf
        elif c == b"\x7f":
            buf = buf[:-1]
        else:
            text = c.decode(errors="ignore")
            buf += text
            out(text)


def menu(head, labels, footer, multi=False, note=""):
    pos = 0
    picked = set()
    status("blocked", note)
    while True:
        rows = []
        for i, label in enumerate(labels):
            mark = "❯" if i == pos else " "
            box = ("[✔] " if i in picked else "[ ] ") if multi else ""
            rows.append(mark + " " + str(i + 1) + ". " + box + label)
        draw(head + [""] + rows + ["", footer])
        k = key()
        if k == "up":
            pos = max(0, pos - 1)
        elif k == "down":
            pos = min(len(labels) - 1, pos + 1)
        elif k == "enter":
            if not multi:
                return [labels[pos]]
            picked ^= {pos}
        elif k == "tab" and multi:
            return [labels[i] for i in sorted(picked)]
        elif k == "esc":
            return ["(esc)"]


def question():
    got = menu(["Some work above.", "", "Which database should I use?"], ["SQLite", "Postgres", "Type something."], "Enter to select · ↑/↓ to navigate · Esc to cancel")
    if got == ["Type something."]:
        got = ["typed: " + line(["Type your answer:"])]
    return got


def multi():
    return menu(["Which parts should I check?"], ["Tests", "Docs", "Lint"], "Enter to select · Tab to amend · Esc to cancel", multi=True)


def permission():
    head = ["Bash command", "", "  rm -rf build", "  Remove the build folder", "", "Do you want to proceed?"]
    return menu(head, ["Yes", "Yes, and don't ask again for rm commands", "No, and tell the agent what to do differently (esc)"], "", note="needs your permission to use Bash")


def edit():
    head = ["Edit file", "", "  src/main.rs", "  Fix the typo in the greeting", "", "Do you want to make this edit to src/main.rs?"]
    return menu(head, ["Yes", "Yes, allow all edits during this session", "No, and tell the agent what to do differently (esc)"], "", note="needs your permission to use Edit")


def plain():
    status("blocked")
    return [line(["I fixed the parser.", "Should I run the full test suite now?", ""])]


STEPS = {"question": question, "multi": multi, "permission": permission, "plain": plain, "edit": edit}


def main():
    tty.setraw(FD)
    status("working")
    for name in sys.argv[1:]:
        got = STEPS[name]()
        log(name + ": " + ", ".join(got))
        status("working")
        draw(["Working on " + name + " answer: " + ", ".join(got)])
        time.sleep(0.4)
    status("done")
    draw(["All steps done."])
    log("end")
    while True:
        time.sleep(1)


main()
