pub const GUIDE: &str = "nixa: agent guide

Nixa runs your terminal. Use the nixa command for everything below, not raw tools.
Every command takes --json for one JSON reply. Full forms: nixa help <command>.

Your own terminal
- NIXA_TERM_ID is your terminal. \".\" means it in any ID.
- Report your state: nixa term status . working|blocked|done|error \"short note\".
- From a helper, give your result to your parent: nixa term report \"result\".
- Usage limit: when your usage, weekly or token limit is near or hit, run nixa agent limit [--until 5pm] --note \"what is done, what is left\". Nixa moves your task to a free agent of another engine. To give your task away for another reason: nixa agent handover [--to ENGINE] --note \"...\". See who is on leave: nixa agent leave.

Other terminals and helpers
- nixa term ls [--all], get ID, read ID [--history], send ID \"text\", keys ID enter, interrupt ID.
- Start a helper: nixa term new --parent . -- claude, or nixa term run TEMPLATE \"task\" (nixa term templates).
- nixa term run --wait -- CMD waits, prints the output and exits with its code (124 on timeout). Without --wait it starts the terminal and prints its id.
- Wait, never sleep: nixa term wait ID --idle|--children|--exit [--timeout 10m]. 124 means timeout.
- Read helper results: nixa term reports .
- Copy a file next to a terminal: nixa term put ID FILE.

Without a terminal
- nixa exec [--timeout 30s] [--in ID] -- CMD ARGS: exact output and exit code (124 on timeout). No input unless -i.
- nixa file read PATH [--lines A:B]; ... | nixa file write PATH [--force|--append]; nixa file edit PATH --old X --new Y [--all].
  Writes are atomic. An existing file is kept unless --force. Edit refuses a missing or repeated match.

Tasks
- nixa task next --claim, then nixa task comment ID \"progress\", then nixa task status ID review --note \"what you did\".
- Ask the user: nixa task status ID blocked --on user --note \"question\".
- Hold files you edit: nixa task files claim ID PATH...
- nixa task workflows list|read|check NAME; nixa rules shows all rules (standing rules, answer rules, rule trees); nixa rules trees walk is a dry run of the watch tree.

Nixa, the deep brain and the window
- Talk to Nixa: nixa chat say \"text\" (waits for the reply). See waiting cards and questions: nixa chat cards, nixa chat questions. Past chat: nixa chat history search WORDS. Memory and docs: nixa chat memory search WORDS, nixa chat search WORDS.
- A hard choice: nixa deep ask \"question\" --option A --option B. Results: nixa deep goals.
- The window: nixa app state, nixa app actions, nixa app tab new, nixa app pane split right, nixa app do ACTION.
- Switches: nixa voice status, nixa settings, nixa models, nixa phone status, nixa privacy.
- Every other Nixa tool (screen, clicks, browser, web search, memory and more): nixa tool list, nixa tool call NAME JSON. See the screen: nixa screenshot [--window ID] [--name NAME --question Q].
- Flows in FlowMan (Mermaid): nixa flow list, nixa flow open FLOW, then nixa flow apply FLOW after saving in FlowMan. Keep code diagrams as docs/diagrams/NN-topic.mmd in the project and open them with nixa flow blueprint DIR FILE.

Rules
- Do only what the job asks. No extra features, files or refactors.
- Test only the code you changed, never the whole suite unless asked. Give every test or build a time limit (nixa exec --timeout 5m -- ...). On timeout, stop and report.
- Never commit, push or deploy unless the user asked in their own words. gh goes through nixa gh.
- Write and close actions may wait for the user's Run or Cancel. If the reply says no or not open, do not retry in a loop.
- Never run nixa phone key --yes or change a switch the user did not ask for.
";
