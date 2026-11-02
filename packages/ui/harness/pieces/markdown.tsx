import { Markdown } from '../../src/kit'
import type { Fixture } from '../fixture'

/** A guide: headings with anchors, an in-page link, a link to another guide, an image, a table. */
const GUIDE = `# Widgets

A widget is one thing the harness keeps track of. This guide covers [sharing one](#sharing-a-widget), and the [first steps](example.md#first-steps) guide starts from the beginning.

## Sharing a widget

Open the widget and choose **Share**. The menu is shown in ![the share menu](img/share-menu.png).

### Who can see it

| Role | Can see it | Can change it |
| --- | --- | --- |
| Owner | Yes | Yes |
| Editor | Yes | Yes |
| Viewer | Yes | No |

## Removing a widget

1. Open the widget.
2. Choose **Remove**, then confirm.

Removing a widget cannot be undone.
`

/** Instructions an admin wrote: raw HTML stays text, links open in a new tab, images stay links. */
const INSTRUCTIONS = `## Before you start

Make sure the widget can reach \`widgets.example.test:7443\`. The <b>full</b> list of ports is in the [network notes](https://widgets.example.test/docs/network).

<script>alert("hello")</script>

- Install the agent.
- Paste the join token when it asks.

![the network diagram](https://widgets.example.test/img/network.png)

[Do not follow this](javascript:alert(1))
`

const fixture: Fixture = {
  id: 'markdown',
  states: {
    guide: () => <Markdown text={GUIDE} guideLinks />,
    instructions: () => <Markdown text={INSTRUCTIONS} />,
  },
}

export default fixture
