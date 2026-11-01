# 0.44.3

## Fixed

- The board, a session's window and Chat no longer flicker when they update. The card under the pointer keeps its highlight, a code block keeps its Copy button while you hover it, and a text selection or tooltip is no longer lost when a new turn arrives.

# 0.44.2

## Fixed

- Maya's window and the phone app start with less to load: the live terminal's code now loads only when a live terminal opens, so the phone never loads it.
- While a session's window shows its live terminal, Maya no longer reads the session's history again at each board refresh. It reads it again when you switch back to the chat look.
- A live terminal that is open when you change Maya's theme now takes the new theme's colours at once.
- In the light theme, muted text in a live terminal, such as zsh's suggestions, is grey again and not nearly black.
- When a Native session that reviews a pull request ends before it shows on the board, Maya names the pull request's folder, not the clones folder. The question before a quit names it the same way while the review is still starting.
- The question before a quit now shows even when part of Maya's window fails to start, and if "Quit anyway" fails, Maya says why.
- The question before a quit has a title, and Tab and Shift+Tab stay on its two buttons.
- A reply, an answer or a Stop that Maya types into a Native session now goes in after the keys you typed in its live terminal, not between them.
- A program that a Native session's terminal leaves running now ends with that terminal.
- If Maya cannot set up a Native session's terminal, the program it started no longer keeps running, and an error opening the terminal shows at once.
- A Native session that stops reading what is typed into it no longer holds up replies, answers and Stop for your other sessions, or the board. After 5 seconds Maya says that the session isn't reading its input.
- Rare cases where Maya could signal a process that was not hers, leave a live terminal at two sizes, or close without stopping her Native sessions are fixed. Finding which sessions are Native also takes less work at each board refresh.

# 0.44.1

## Fixed

- On the phone, a session opens under the top bar, like Chat, and tapping its reply box brings up the keyboard. The new-session and resume dialogs open the same way, and tapping a tab closes them.

# 0.44.0

## New

- Sessions can now run inside Maya, on macOS, Windows and Linux: choose Native in Settings › Sessions › New sessions open in. A Native session opens no terminal app; the other choices work as before.
- In the terminal look, a Native session's window is its live terminal and takes every key, including arrows, Esc, Tab, Ctrl+C, paste and slash commands such as /config. Copy and paste are Cmd+C and Cmd+V on macOS, and Ctrl+Shift+C and Ctrl+Shift+V on Windows and Linux. In the chat look, the reply box and the answer menu work as before.
- A card's Terminal button opens a Native session's window, in the look you last chose. In the terminal look, that is its live terminal.
- A Native session you start with the + button, resume on its own, or start to review a pull request opens its live terminal at once, before it shows on the board. A question such as whether to trust the folder, a login, or an error shows there.
- If a Native session ends before it shows on the board, Maya says which folder it was in and shows the last lines it printed.
- On Linux, a Native session takes typed keys without tmux.
- Native sessions stop when Maya quits. Quitting Maya, closing its window or restarting to update asks first when a Native session is working, or is still starting and not yet on the board. Logging out or shutting down is never held up.
- Native sessions that were open when Maya quit are offered back the next time it starts, in the Sessions tab or by voice ("Maya, resume my sessions"), and come back as Native.
- For now, another machine's Maya and the phone show a Native session as its chat and transcript, not as its live terminal.

# 0.43.0

## New

- Settings has a Release Notes page, on the desktop and on the phone, listing what each version of Maya brought, newest first.
- The phone's Network tab is now Settings: a list of groups (Network, Notifications, Voice, Session viewer, Release Notes), each opening its own page. Android's Back returns to the list.
- Each release on GitHub now opens with its release notes.

# 0.42.0

## New

- Every code block has a Copy button in its top-right corner, shown when the pointer is over it and always on a touch screen.

## Fixed

- Long lines in a code block wrap, so the block no longer covers the line below it, in the session window and in Chat.

# 0.41.0

## New

- A session's window shows small meters in its header: how full the context is, how much of each usage limit is used and when it resets, and the cost so far.
- The meters shrink with the window, and hovering shows the full numbers, the reset time and the account.
- Settings → Sessions has Install usage status line, which lets Claude Code sessions show their limits and Claude Code's own cost. Your own status line stays as it was.
- The `maya` command installs the usage status line too, with `maya statusline install`.
- Without the usage status line, a Claude Code session's cost is estimated from its tokens and marked "est.".
- Codex, Antigravity, Kiro CLI, Grok Build and OpenCode sessions show the usage and limits each agent shares.
- In the terminal look, the bottom line is the session's own Claude Code status line, in its colours.
- Sessions from paired machines show the same numbers on the main.

# 0.40.2

## Fixed

- On Windows, pairing and connecting to another Maya no longer fail when the other end is slow to answer.

# 0.40.1

## Fixed

- On phones where it still sat under the keyboard, a session card's reply box now stays above it.

# 0.40.0

## New

- New sessions can start in their own git worktree: tick New worktree, and the session gets its own folder on a new branch off the branch your checkout is on.
- While New worktree is ticked, the form warns that a closed worktree session cannot be resumed from the Resume list.
- A problem with the worktree, such as a folder that is not a git repository, shows in the form instead of in a terminal.
- The `maya` command starts a session in a worktree too, and a paired machine makes the worktree in its own folders.
- Pull-request reviews no longer use your own project checkout. Each repository is cloned once, and every review gets its own worktree of that clone.
- Pressing Review on a pull request whose review is still running says so instead of opening another terminal.
- Reviews work with OpenCode powering Maya for repositories that are not checked out under the projects directory.

## Fixed

- In the terminal look, your own messages sit on a lighter row, so they stand out between the agent's turns.

# 0.39.2

## Fixed

- On the phone, a session card's reply box stays above the keyboard.
- On the phone, a password manager's popup no longer sits over the reply box, takes the focus and makes the keyboard flicker.

# 0.39.1

## Fixed

- Fewer status reports are wrongly put in Awaiting Decision: a session counts as asking only when it has stopped because it needs something from you.
- Every finished message is judged again under the new rule.

# 0.39.0

## New

- On the phone, pull a notification down to read the whole question with its choices, or the whole last message of a finished turn.
- A question's first choices are buttons on the notification (up to three), numbered when a choice is too long to fit.
- A finished turn, or a question asked in plain words, takes a reply straight from the notification.
- When an answer from a notification fails, the notification comes back and says why.

# 0.38.1

## New

- A session whose last message asks you something in plain words stays in Awaiting Decision until you reply, instead of moving to Completed.
- Maya asks her own agent whether a finished message asks you something, and remembers the answer for that message.

## Fixed

- A session that stopped before reporting its last turn no longer shows Idle when its final message asks you something.
- On the phone, the box you type in stays above the keyboard, and the new-session form scrolls so its prompt, options and Start stay in reach.
- On the phone, the top bar fits the screen with the Chat tab, so full-screen cards and dialogs are no longer cut off at the right.

# 0.37.0

## New

- Maya's replies in Chat show as formatted text (lists, tables, code), and their links open in your browser.
- Aloud, Maya says her replies as plain words, kept short, and puts details such as paths on screen.

## Fixed

- Chat opens at the latest message.
- A web address in brackets or quotes is said as "a link" instead of letter by letter.
- Maya's announcements keep your own words as you wrote them.

# 0.36.0

## New

- A Deployments tab, next to Pull Requests, lists the GitHub Actions runs that wait for your approval, with Approve and Reject buttons.
- A run held behind another deployment shows under the run it waits on, with a link to that run.
- Maya checks for waiting deployments every two minutes, on the main or a Maya on its own.
- When the GitHub token lacks the permissions the Deployments tab needs, the tab names the permissions to add.

## Fixed

- On Windows, Maya's voice from your own speech server plays instead of failing.

# 0.35.1

## Fixed

- On the phone, a session window's header wraps onto more lines instead of squeezing the name, folder, model and branch over the state badge.
- On the phone, a password manager's autofill popup no longer appears while you type a reply and closes the keyboard.

# 0.35.0

## New

- The phone has a Chat tab: the main computer's Chat, with the same conversation, including the turns that were spoken.
- A message typed in the phone's Chat gets Maya's reply on both screens, and Cancel and Stop work as they do on the computer.
- The Chat tab shows only while the phone is joined to a main that lets it see and control the sessions.
- The phone now only joins a main computer. It can no longer be the main, and the computer no longer offers Make main for a phone.
- A phone that was the main opens with networking off and the Join screen. Choose "Act as main Maya" on one computer, then join the phone to it.
- The phone's first screen is Join, and its Network settings drop the pairing code, the paired machines, the port and Make main.
- The phone's persistent notification names the computer it is joined to ("Joined to desk", "Reconnecting to desk…").

## Fixed

- When a Chat message cannot be sent, Maya says so and puts the text back in the box.
- The phone shows why it could not reconnect to the main at start-up.
- The phone's Chat clears when the main removes the phone, turns its switch off, or cannot chat, so another main's conversation no longer shows.
- Send, Cancel and Stop in the phone's Chat are easier to tap.

# 0.34.1

## Fixed

- Files you paste or drop into a session's reply box now reach the session. Before, Maya showed the file, but the session never received it.

# 0.34.0

## New

- Settings › Keyboard chooses which Enter combinations send a message and which make a new line, with as many as you like for each. The setting applies to the Chat tab, a session's reply box and the new-session prompt.
- By default, Enter sends. Option+Enter and ⌘+Enter make a new line on a Mac, and Alt+Enter and Ctrl+Enter do on Windows and Linux.
- With the defaults, a session's reply box and the new-session prompt now send on Enter, and ⌘+Enter makes a new line.
- After you update, a notice tells you about this change once, with a button that opens Settings › Keyboard.
- The hints by each box show the key that sends in your setup.

# 0.33.0

## New

- Code blocks in the session viewer have colours by language, such as shell, SQL, TypeScript, Python and Rust. A block with no language stays plain.
- Inline code shows in its own colour.
- In the light theme, the viewer's terminal look is light too.

# 0.32.0

## New

- Maya has a light theme. Choose it in Settings › General › Theme. Dark stays the default.
- A light theme you saved shows from the moment Maya opens, with no flash of dark.

# 0.31.1

## Fixed

- On a Mac without iTerm2, replies to sessions in Terminal reach the terminal again. Since 0.30.0 they failed with "Part of the reply was typed into the terminal".

# 0.31.0

## New

- Maya can speak through a voice server that you run yourself, if it works like OpenAI's speech service, for example Chatterbox.
- Maya speaks with the first voice that works, in this order: ElevenLabs, then your voice server, then the built-in voice. A voice that fails is skipped for 5 minutes.
- In Settings, each remote voice has its own switch, in place of the single voice menu. The server's address, voice, model and key are set on each device, and the key is kept in your system's key store.
- "Try the voice" tells you which voices failed and which one spoke.
- Updating does not change the voice you hear.
- The project includes a ready-made Chatterbox server and a guide to running it.

# 0.30.1

## Fixed

- Maya can reach sessions whose terminal runs a shell wrapper, such as Kiro CLI's shell integration. Before, bringing the session forward, replying and Stop failed with "No Terminal tab found". This also applies to a wrapped shell in tmux on Linux.

# 0.30.0

## New

- Maya works with sessions in iTerm2. She can open, type into, bring forward and stop them, in any window, tab or split pane.
- On a Mac, Settings › Sessions › "New sessions open in" chooses Terminal or iTerm2. Terminal stays the default.
- Each new session opens in its own iTerm2 window.
- Stop works for iTerm2 sessions without Accessibility permission.

# 0.29.0

## New

- A Stop button on a working session does what Esc does in its terminal: the agent ends its turn and the session stays open. It works for Claude Code, Codex and OpenCode sessions.
- On a Mac, the first Stop can ask for Accessibility permission.
- When a session asks a question, the reply box becomes a menu like the agent's own. Pick an option, choose "Type something…" to answer in your own words, or choose "Chat about this".
- Stop and the new answers work from the laptop and the phone too.
- By voice, "stop \<session>" stops a session, and a spoken reply to an open question counts as an answer in your own words.
- Permission and plan menus, and questions that take more than one answer, are still answered in the terminal.

# 0.28.0

## New

- Codex, Grok, Kiro, Antigravity and OpenCode can now be Maya's long-running assistant brain too, like Claude Code. She remembers the conversation, does jobs herself and speaks up about her results.
- Maya never approves a permission request for the agent that is her brain.
- Her OpenCode brain session does not show on the board.
- Some limits: with Grok or Antigravity, the background helpers she starts cannot be listed or stopped, and with Codex she has no network access.

# 0.27.0

## New

- Maya's brain is now an assistant that remembers the conversation, between requests and across restarts. This works when Claude Code is her brain.
- She knows which session has just finished or started asking, so you do not always have to name it.
- She can send work to a session, start a new session in a repository or in a scratch folder, or do a small job herself and report back.
- She no longer asks "Yes?". She says what she will do and does it after 5 seconds. Say stop or no, correct her, or press Cancel to stop it, or say "yes" to run it at once.
- "Maya, stop" stops the work she is doing herself.
- She speaks up on her own about her own results and about the things you asked her to watch.
- A new Chat tab lets you type to her as well as talk. It has a status dot, the 5-second countdown with Cancel, and Stop while she works.
- The phone also runs announced actions after 5 seconds. When a request takes a long time, the phone says that the answer will be in Chat.
- On an assistant, Chat is read-only. When you make a different computer the main, it catches up on what happened on the old main.
- Settings → Debug (off by default) shows the Debug tab and her own card.
- When her session is down, and for brains other than Claude Code, she still answers in the quick one-request way she used before.

# 0.26.0

## New

- You can choose the wake word in Settings → Voice → Listening. If you leave it blank, the word is "Maya".
- Only the word she answers to changes. She still calls herself Maya.
- "\<your word>, yes" confirms, the same way "Maya, yes" does.
- The mic button, the voice panel and the listen switch show the word you chose.
- A phone uses its main's wake word, and gets a new word without reconnecting.

# 0.25.1

## Fixed

- A Claude Code session that ends its turn with a question now moves to Awaiting Decision while a background command it started is still running. Before, it stayed in Working until that command ended.

# 0.25.0

## New

- After a restart, the Sessions tab offers back the sessions that were running, with Resume all, Choose… and Dismiss. This works on the desktop and on the phone.
- The offer covers your paired machines too. Each session reopens in its own folder and terminal, on its own machine.
- "Maya, resume my sessions" does the same by voice.
- When a session cannot be resumed, Maya says why, for example that its folder is missing.
- A session you closed from its card is not offered after a restart. An offer you ignore goes away after a day.
- The session viewer shows what the terminal shows: each command's output, edit diffs with line numbers, how long the session has been working, and a status line with its model, folder, branch, pull request and context.
- The viewer has a terminal look that is like the agent's own terminal. Choose it in Settings → Sessions or with the `>_` button in the viewer.

## Fixed

- A session that has read screenshots no longer shows an almost empty history in the viewer.
- The viewer keeps a message that has a pasted image, and the reply before it.

# 0.24.0

## New

- Settings → Git turns GitHub and GitLab on or off, each on its own, so you can use both, either or neither.
- Each one takes a token. You can paste it, and Maya keeps it in your system's key store, or you can give the name of an environment variable that holds it.
- Maya no longer needs the `gh` command-line tool.
- GitLab merge requests show in the Pull requests tab beside GitHub pull requests, each with its site's mark. A session in a GitLab checkout shows "MR !12" on its card.
- A self-hosted GitLab works at its own address.
- A Test button for each provider says "Connected as @you" or tells you what is wrong.
- You can choose SSH (the default) or HTTPS for the clones Maya makes for reviews.
- When one provider fails, the tab keeps its last results and shows an error line for that provider.
- When you update, GitHub stays on. If `gh` is logged in, Maya takes its token, so you have nothing to set up.

# 0.23.0

## New

- The phone can talk to Maya. Tap the mic and speak, and your computer (the main) works out what you mean.
- Before it acts on a spoken request, the phone reads back what it will do and waits for a yes or a no, spoken or with the Yes and No buttons.
- The phone can listen for the wake word "Maya" (experimental). Turn it on in the Voice card on the Network tab.
- While the app is open, the phone says aloud when a session needs a decision or has finished. This happens only when Speak is on and the phone is not on silent, vibrate or Do Not Disturb.
- Spoken alerts come without a notification sound, so nothing plays over the speech.
- A mic bar on the phone's board shows what it heard and what it said. When microphone access is off, the bar has a button that opens the app's settings.
- Voice from the phone never brings a window forward or opens anything on the computer.

# 0.22.1

## Fixed

- Settings no longer leaves large gaps or mixes unrelated settings. It is now a sidebar of six groups (General, Sessions, Pull requests, Notifications, Voice and Network) that shows one group at a time and remembers the last one.
- Voice holds both speaking and listening, and Pull requests holds all the pull request settings.
- On a narrow window, each setting's label sits above its control.

# 0.22.0

## New

- Everything sent between Mayas is now encrypted and authenticated, and pairing uses the code without ever sending it.
- Every Maya has to pair again after this update; earlier pairings stop working. Maya says which ones need it.
- Settings › Network has New key, to give this Maya a new identity, and Forget main. The `maya` command has `pair --new-key`.
- Android release APKs are now signed with one key, so each new one installs over the one before.

## Fixed

- Make main's confirmation tells you to pair an offline assistant with the new main, instead of saying it needs no new pairing.
- On Linux, the `.deb` now installs the key-store tool Maya needs to start its network.

# 0.21.0

## New

- The phone can join a Maya that is already the main, and shows that main's board: every session on every machine.
- From the joined phone you can reply, answer and start sessions on any machine, and get a notification when one needs you.
- On first start, the phone offers Be the main or Join a main. Its Network tab can join a main, and shows which main it is connected to with a Leave button.
- The main has a switch for each phone, "Sees and controls all sessions", to allow or stop it.
- A phone that hands its main role away now keeps showing every session, instead of an empty board.

# 0.20.0

## New

- A search box over the Sessions board narrows the columns as you type, by repository, session name, agent or machine. Cmd+F (Ctrl+F) focuses it and Escape clears it.

# 0.19.1

## Fixed

- Multi-line replies to Claude Code are sent in full, instead of sometimes keeping a stray `\` and not being sent. This happened most on Windows.

# 0.19.0

## New

- The Pull Requests tab splits into Review Requested and Assignee, each with State and Repository filters. Maya remembers the last one you opened.
- Pull requests you have reviewed stay under Review Requested until they close, with a dot for your review: green for approved, blue for commented, red for changes requested.
- Under Assignee, the dot shows what the reviewers decided, with their names.
- Maya tells you when a pull request assigned to you is approved or needs changes, with a notification and, if speech is on, out loud. A setting turns this off.
- The tab's count shows only the pull requests waiting on you, and the spoken summary says which ones you reviewed and who reviewed yours.
- A pull request whose review is requested again counts as one to review.

# 0.18.0

## New

- Maya updates itself. A bar on the board offers each new release, and Settings has Check for updates.
- Where Maya cannot update itself (an `.msi` or `.deb` install, and the phone), the bar offers a Download link instead.

# 0.17.0

## New

- Settings has a Voice volume slider, from 10% to 100%. It applies to everything Maya says and to Try the voice; the notification sound keeps your system's volume.
- Maya's voice is quieter than before: the slider's 100% is softer than the old fixed volume.
- Try the voice now says "Maya here. Is my voice ok?"

# 0.16.0

## New

- The main can hand its role to a connected assistant with Make main in Settings › Network, and the other Mayas follow the new main without pairing again.
- Make main asks you to confirm first, and names any Maya that cannot follow or is offline.
- The phone can hand the main role over too. It then shows "Listening to" the new main with a Stop listening button, and can be made the main again later.
- The `maya` command follows the main when it moves to a new machine.

# 0.15.0

## New

- Replies you send through Maya are typed into the Claude Code session's terminal, so Claude Code takes them as your own words. A reply from the phone or another machine can now approve what the session asks.
- The reply box says "Sent as you", or warns when a reply could only go in as a message from another session, which cannot approve anything. This happens when the session runs in a terminal Maya cannot type into, such as iTerm, VS Code or Ghostty.
- While Claude Code shows a choice, Maya asks you to answer it before sending a reply.
- A session's history shows compactions and interrupts as short notices, shows pasted text without its wrapper, and labels messages from other sessions as such.

# 0.14.2

## Fixed

- The Android app shows Maya's icon instead of the Tauri logo.

# 0.14.1

## Fixed

- The Pull Requests tab works when Maya is opened from the Dock and your GitHub token is set in your shell's startup file, instead of asking you to run `gh auth login`.

# 0.14.0

## New

- Settings is regrouped into a grid of sections: Maya, Sessions, Notifications, Network and Voice assistant. It drops to two columns, then one, on narrower windows.
- Settings shows the version of Maya you are running.
- When this machine is the main, Settings › Network lists its addresses, so you know what host to type on an assistant. The list updates when the network changes.

# 0.13.0

## New

- Maya comes as an Android app. The phone is a main Maya: pair your computers with it and their sessions show on the phone's board.
- Upright, the phone shows one column at a time; swipe or tap the strip at the top to move between them. Sideways, it shows up to four.
- From the phone you can reply, answer a decision, attach a file and start or resume a session on a paired computer.
- The phone posts a notification when a session needs a decision or finishes a turn. Tapping it opens the session's card, even if Android had closed Maya.
- Maya keeps running on the phone with the screen off, behind a persistent notification. Keep Maya awake exempts it from battery optimisation.
- The phone's Network tab shows the pairing code and the phone's Wi‑Fi address, starts and stops the server, and turns the notifications on or off.
- Each release has an APK for the phone to download.

## Fixed

- A file over 20 MB is refused as soon as you pick or paste it, instead of being read first, which could crash the window.

# 0.12.1

## Fixed

- The Pull Requests tab and the cards' pull request buttons work when Maya is opened from the Dock or Finder.

# 0.12.0

## New

- OpenCode sessions show on the board, and OpenCode can power Maya.
- You can start, name, resume, reply to and close OpenCode sessions from Maya.
- OpenCode permission prompts and questions can be answered from the card.

# 0.11.0

## New

- Kiro CLI sessions show on the board, and Kiro CLI can power Maya.
- You can start, name, resume and reply to Kiro CLI sessions from Maya.
- Close works for every agent, not only Claude Code.

## Fixed

- The session window shows the model and effort pickers for Codex, Antigravity and Grok Build sessions.
- A main no longer freezes an assistant's cards when the assistant has a session from an agent the main does not know; that card shows as "Unknown agent".

# 0.10.0

## New

- You choose which agent powers Maya: Claude Code, Codex, Antigravity or Grok Build. Maya asks on first start, and Settings can change it.
- Maya's agent is the default for new sessions, resumes and reviews.
- You can resume past sessions of every agent, not only Claude Code.
- Compact, model, effort and mode controls work for the other agents where the agent supports them.
- Reviews run with the agent you choose, on a review prompt you can edit in Settings.

# 0.9.0

## New

- A line that starts with `!` runs as a shell command in a Claude Code session.

## Fixed

- The card's chat shows only your messages and the agent's answers, without subagent reports, notifications, command echoes or progress notes.

# 0.8.0

## New

- New sessions can start with Codex, Antigravity or Grok Build as well as Claude Code.
- New sessions can be given a name, which the card shows at once.
- The model, effort and mode choices follow the agent you pick.
- A main can start any agent installed on an assistant.

## Fixed

- You can rename a session of any agent from the session window, not only Claude Code.
- A Codex, Antigravity or Grok Build card shows the new name after a rename in the agent, without restarting Maya.

# 0.7.0

## New

- Idle and Completed Claude Code cards have a Close button that exits the session and closes its terminal.

# 0.6.0

## New

- A mute button in the top bar stops Maya speaking; banners still show, without a sound.
- When Maya cannot see your Focus mode on macOS, Settings says so and opens Full Disk Access for you.

## Fixed

- With Full Disk Access granted, Maya stays quiet under Do Not Disturb; before, she could keep speaking.

# 0.5.1

## New

- A message that starts with `/` runs as a slash command in the session instead of going to the model as text.
- The app icon shows how many sessions wait for a decision, across every machine on the board.
- A new decision makes the app icon ask for attention once.
- The session window takes most of the screen and resizes from any edge or corner, and remembers its size.

# 0.4.0

## New

- Maya runs on Linux (Ubuntu with GNOME, on Wayland or X11), voice included, as a `.deb` or an AppImage.
- On Linux, sessions Maya starts run in tmux, in a terminal window.

## Fixed

- Removing the Codex hook works when Codex's hooks file is empty.

# 0.3.0

## New

- Maya runs on Windows 10 and 11, voice included.
- On Windows, sessions open in Windows Terminal running Git Bash.
- Settings can install a Codex hook, and each hook button names its agent.
- Your own replies show as "You" in the conversation.

## Fixed

- Sessions in project folders whose names contain `.` or `_` show on the board on macOS.

# 0.2.0

## New

- Several Macs on one board: a main Maya shows the sessions of every assistant Maya paired with it.
- An assistant pairs with a main once, with a six-digit code that the main shows.
- From the main you can reply to, answer, start and resume sessions on an assistant.
- Notifications and voice commands on the main name the machine a session runs on.
- Settings › Network sets a Maya as a main or an assistant.
- A headless `maya` command runs an assistant on a machine with no desktop, such as a Linux box or a Mac over SSH.
- Sessions the main starts on a headless machine run in tmux, and the main shows how to attach to them.

# 0.1.0

## New

- Maya shows your Claude Code sessions on one board, in Idle, Working, Awaiting Decision and Completed columns.
- Codex, Antigravity and Grok Build sessions show on the board too, each with its agent's icon.
- Each column's count shows on the Sessions tab.
- A session that ends its turn with a question shows as Awaiting Decision.
- Settings installs the Claude hook, which makes Awaiting Decision and Completed exact.
- Clicking a card brings its Terminal window forward.
- You can reply to a session from its card, and read its conversation, with formatting, in the session window.
- Replies can carry files and images: drop them in or paste them.
- You can answer a session's question by picking one of its options on the card.
- The "+" starts a new session in a project folder, with the model, effort and mode you choose.
- You can change a running session's model and effort, and cycle its mode, from the session window.
- You can rename a session from the session window's title.
- You can resume a past session from the board.
- Each card shows how full the session's context is, with a Compact button from 75 percent.
- Cards show a button for the session's pull request when it has one.
- The Pull Requests tab lists the pull requests that ask for your review or are assigned to you, and reviews one in one click.
- A pull request card links to the session working on it.
- A notification tells you when a session needs a decision.
- Maya can speak instead of playing a sound: she says when a session needs a decision or is finished.
- An ElevenLabs voice is optional.
- Maya stays silent while a Focus mode is on.
- Voice commands: say "Maya, what's waiting on me?" or "Maya, tell \<session> to go ahead", and she answers aloud and acts on the board.
- Actions you give by voice are read back and wait for your "yes".
- Speech recognition uses Apple's recogniser, or a built-in Whisper model for Macs where Dictation is turned off.
- Maya installs on macOS with one command.
