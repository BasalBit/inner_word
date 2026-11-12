# Recordings of Claude Code

Output of the real `claude` program (Claude Code 2.1.322), recorded on 2026-11-12 and
replayed by the stand-in program of the process tests (`tests/fake_claude`). Everything
that identifies the user, the account or the machine was replaced (paths, the e-mail
address, organisation and ids, dates of the sign-in); session ids and message ids are
the random ones of the recording.

Recorded with the user's own sign-in and configuration in place, against the real
service (five one-line turns on the cheapest model at low effort, no session kept):

| File | What it is |
|---|---|
| `auth-status-signed-in.json` | `claude auth status`, signed in with a claude.ai subscription |
| `initialize.jsonl` | the answer to an `initialize` request (no message sent): models, effort levels |
| `quick-turn.jsonl` | one turn with the quick chat's flags: no tools, our system prompt |
| `side-tool-turn.jsonl` | one turn with the side chat's flags: the library's MCP server, one `read_document` call, thinking |
| `three-turns.jsonl` | one process, three turns: an answer, an answer interrupted mid-stream, a follow-up |
| `interrupted-turn.jsonl` | the interrupted turn of the above on its own |

Recorded from the real program with an empty, throwaway configuration directory (nothing
reaches the service):

| File | What it is |
|---|---|
| `auth-status-signed-out.json` | `claude auth status`, not signed in (exit code 1) |
| `signed-out-turn.jsonl` | a turn while not signed in |

Recorded from the real program with a throwaway configuration directory and dummy
credentials against a stand-in for the service on localhost, which answers as the
service does in each case (the program's own words and shapes are real, the service's
are made up; "127.0.0.1" in a sentence is that stand-in):

| File | The stand-in answered |
|---|---|
| `tool-turn.jsonl` | a `read_document` call, then text |
| `withheld-tool-turn.jsonl` | a call to a tool that writes (`append_to_document`), then text |
| `error-usage-limit.jsonl` | 429 with the headers of a subscriber's used-up five-hour allowance |
| `error-overloaded.jsonl`, `error-overloaded-retries.jsonl` | 529, without and with four retries |
| `error-server.jsonl` | 500 |
| `error-sign-in-expired.jsonl` | 401 |
| `error-model.jsonl` | 404 for the model |
| `error-cut-stream.jsonl` | a stream that ends in the middle |
| `error-no-connection.jsonl`, `error-no-connection-retries.jsonl` | nothing listening, without and with four retries |
| `set-model.jsonl` | two turns with `set_model` and `apply_flag_settings` between them |
