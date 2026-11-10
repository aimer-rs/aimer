---
name: aimer-app
description: Use Aimer MCP's session tools to remotely run, drive, and observe a live Aimer app. Look at the app with screenshots and logs, operate it with mouse, keyboard, and touch input, restart it after code changes, and read its resource usage.
---

# Aimer App

Use this workflow to work with a running Aimer app from the outside: start or restart it, look at it, operate it like a user, and check what it did. You act through Aimer MCP tools; you never need to touch the app's window yourself.

## Ground rules

- Call Aimer tools through the agent's native MCP tool interface. Never run `aimer mcp` as a subprocess or write JSON-RPC to call it. If the tools are not in the tool list, load them with the agent's tool-search step if they are deferred; otherwise say so.
- Input tools are listed only while an app is connected to the attached session and accepts input. If `left_click`, `type`, and `screenshot` are missing, the app is not running, not attached, or is a release build. Check `status` and `list_sessions` instead of guessing.
- Drive only debug builds started by `aimer run`. A release build never accepts input.
- While you hold the app, it shows a `driven by aimer-cli` badge and draws each injected pointer and finger. Do not treat the badge or pointer overlay as part of the app's own UI.
- Only act on what the user asked for. Do not click through destructive or irreversible flows (delete, purchase, sign-out, send) unless the user requested them.

## Find and choose the session

1. Call `status` for the run state, process ID, runtime, and target device.
2. Call `list_sessions` to see live sessions and the connection mode: `automatic`, `pinned`, or `detached`. Add `all_projects` to see sessions of other projects.
3. With exactly one live session, the connection attaches to it automatically. With several, call `attach_session` with a `session_id` to pin one. `attach_session` without an id returns to automatic mode; `detach_session` controls nothing until the next attach.
4. These calls change only your own connection and do not move other agents sharing the workspace.

## Start, restart, and stop

- Use `restart_project` after code changes. It does a clean rebuild and waits for the new run; then check `status` and `get_logs` for build errors before driving the app.
- Use `stop_project` to end the session when the user is done or asks for it.
- If the app was started with `aimer run` outside your control, attach to it rather than starting another.
- The tool list refreshes when the session or its app changes. A client that ignores the notification sees the change on its next listing.

## Look before you act

- Use `screenshot` to see the app's window exactly as the app drew it, with nothing else on screen. Take one before the first action so you know the actual state.
- By default one picture pixel is one logical pixel, so coordinates in the picture can be passed straight to the input tools. If you shrink the picture with `max_width`, divide by the scale the reply reports before using coordinates. `jpeg_quality` makes images smaller.
- Take a screenshot after each meaningful action to check its result. The input tools only report how many events were sent, not what happened.
- Screenshots work where the renderer can copy its frames out (Metal on macOS and iOS today). If the tool says it is unsupported, rely on logs and `get_resource_usage`.

## Operate the app

Coordinates are `[x, y]` logical pixels from the top-left corner of the app's window.

- Mouse: `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click`, `mouse_move`, `left_click_drag`, and `scroll`. A click's `coordinate` is optional: without it the click happens where the mouse already is, once an earlier click, `mouse_move`, or `scroll` has placed it. The pointer glides along a curved path, and a command sent mid-glide retargets it.
- `scroll` is a smooth swipe with `scroll_direction`, `scroll_amount` (steps of 40 pixels), and an optional `coordinate`.
- Keyboard: `key` takes `text` such as `Return`, `Escape`, or `ctrl+a`. `type` types `text` at a human pace; click the field first so it has focus.
- Touch, for apps on a phone or tablet: `tap`, `swipe`, and `pinch`.
- `send_input` sends raw protocol lines for events the tools above do not cover, such as `back invoke`, `link URL`, `file-drop PATH`, `menu`, `preedit`, and `quit`. The whole request is checked before anything is sent.
- Do one action, then verify with a screenshot. Do not chain many blind clicks, because the layout may shift after each one.
- If an input tool fails, read its reason. Common causes: the app never announced an input server, `AIMER_PRENDRE=0` was set for `aimer run`, or the app exited.

## Observe what happened

- Use `get_logs` for bounded build, app, and system logs with source code locations. Page with the returned cursor; do not assume one call returned everything.
- Use `clear_logs` before reproducing an issue so the next `get_logs` holds only the new output.
- Use `get_resource_usage` for the latest app CPU, GPU, memory, PID, and thread sample. Fields can be unavailable on some platforms; the reply gives a reason.
- Use `record_resource_usage` with `duration` (0.5 to 300 seconds) and `interval` (0.5 to 3 seconds) to capture a window while you perform an action. It returns the samples and a recording ID.
- Use `list_resource_recordings` and `get_resource_recording` to read earlier runs, and `set_resource_monitor_interval` (500 to 3000 ms) to change the live sample rate.

## Typical loops

- Verify a change: `restart_project`, `get_logs` for build errors, `screenshot`, operate the changed screen, `screenshot`, `get_logs` for runtime errors.
- Reproduce a bug: `clear_logs`, perform the user's steps with input tools, `screenshot`, `get_logs`, then report the steps, what you saw, and the log lines with locations.
- Check performance: `record_resource_usage` while repeating the slow action, then summarize peaks instead of pasting every sample.

## Without the MCP tools

- In a shell, `aimer input click 120 48`, `aimer input key ctrl+a`, `aimer input type "hello"`, `aimer input scroll 200 300 down 3`, and `aimer input send "back invoke"` do the same as the tools. `aimer screenshot -o app.png` saves a picture, and `aimer sessions` lists live sessions. Pass `--session <id>` when several sessions are live.

## Reporting

- Say which session you drove and what you did, in order.
- Describe screenshots in words and mention only the parts that matter; do not paste raw image data.
- Summarize relevant log lines with their source locations instead of repeating large output.
- State plainly when a step could not be verified, for example when screenshots or input were unavailable.
