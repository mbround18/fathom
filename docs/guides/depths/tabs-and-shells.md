# Tabs, logs and shells

Logs, stats and shells open as tabs at the bottom of the page, so you can watch several containers
at once while you work on a stack.

## Opening a tab

On a stack's page, each container has small buttons:

- **Logs** opens that container's log, or the whole stack's when opened for the stack.
- **Stats** opens its CPU and memory use.
- **Shell** opens an interactive shell in the container. This button only appears when shells are
  turned on (see [Installing Depths](install.md#turning-on-shells)), and a `user` only sees it on
  their assigned stacks.

You can have up to 16 tabs open.

## Panes

On a screen at least 768 pixels wide, tabs can sit side by side in up to three panes. Each pane has
its own tab strip; drag the handle between panes to resize them. From a tab's strip you can:

- move the tab left or right within its pane,
- move it to the pane on the left or right,
- **split** it off into a new pane.

On a narrower screen (a phone, or a narrow window), every tab is in one pane and you switch
between them.

Your tabs and their layout are saved in this browser for your account, so they come back when you
reload or sign in again. Someone else signing in on the same browser gets their own layout, never
yours. Another browser or device starts with no tabs.

## Logs

A log tab opens with the last 1000 lines and then follows new ones. It keeps up to 10,000 lines;
past that, the oldest are dropped and the tab says how many.

- **Follow** jumps to the newest line and stays there.
- **Pause** stops the view moving while lines keep arriving; **Resume** catches up.
- **Wrap lines** wraps long lines instead of scrolling sideways.
- **Search** filters to matching lines.
- **From** and **To** limit the view to a time window.

Control sequences (colours, cursor movement) are removed from log lines, because a container's
output is untrusted and could otherwise rewrite what you see.

When a log stream drops (a proxy cut an idle connection, or the network blinked), the tab reopens
it on its own, from the last line it had, and says **Reconnected**. When the container stops, the
tab says the log ended, and picks up again when the container is running.

## Stats

A stats tab shows CPU and memory for a container, or for each container of a stack, with memory as
a share of the container's limit when it has one. Samples arrive about every two seconds, and only
while the tab is on screen and the page is visible, so idle tabs cost nothing.

## Shells

A shell tab is a terminal in the container, the same as `docker exec -it <container> /bin/sh`.
Depths starts `/bin/sh`, which exists in nearly every image, including small busybox-based ones.

- Hiding the tab, switching to another tab or moving it to another pane keeps the session running,
  and the output is still there when you come back.
- Closing the tab ends the session.
- When a session ends, the tab says when and why (the shell exited, the container stopped, you
  lost access), and offers **New session**.
- A shell cannot survive a dropped connection. If your network drops or Depths restarts, the
  session is gone. The tab says so and offers a new one; it never starts one on its own.
- If your sign-in expires, log and stats tabs reopen after you sign in again; shell tabs wait for
  you to choose **New session**.

Opening a shell is recorded in the audit log (`shell_open`), with who opened it and from where.
What is typed in the shell is not recorded, so treat a shell as full access to that container.
