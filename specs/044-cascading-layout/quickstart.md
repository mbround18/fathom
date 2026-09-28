# Quickstart: Cascading wide-screen layout

## Try it

1. Start Fathom maximised on a ≥ 1920 px display and connect to a cluster. The row is the navigator,
   one section list, and empty columns that say what the next click will open. No second list, no
   Overview pane, no Flux pane.
2. Click **Workloads › Deployments** in a cluster that has something unhealthy. Column 2 is the list;
   column 3 opens the first unhealthy Deployment. Click a healthy section (for example
   **Configuration › ConfigMaps**) and column 3 opens its first row instead.
3. Click a section with nothing in it. Column 2 says the section is empty and column 3 stays empty.
4. From column 3, follow a link to a ReplicaSet, then to a Pod, then to its Node. Each lands one
   column further right and the chain stays on screen.
5. On an ultra display, keep following links past the last column: the oldest step scrolls off the
   left of the cascade while the navigator and the section list stay put. Narrow the window to
   1920 px and back — the same steps are still open, with their scroll positions.
6. Type in a column's search box, scroll it, then resize the window across 1920 and 2560 px in both
   directions. The text and the scroll position survive.
7. Drag a divider, then let another column fill. Your widths are kept.
8. Switch context. The row returns to navigator + section + auto-opened item for the new cluster;
   nothing shows the old cluster.
9. Resize down to laptop, tablet and phone widths. Each behaves exactly as it did before this change,
   and the inspector waits for your click.

## Tests

```bash
pnpm -C ui test -- cascade relevance layout-defaults
cd ui && pnpm exec playwright test tests/e2e/cascade-clean-start.spec.ts \
  tests/e2e/cascade-drill-in.spec.ts tests/e2e/cascade-small-windows.spec.ts \
  tests/e2e/cascade-placeholders.spec.ts --workers=4
```

Then the full suite once, alone on the machine (`free -g` showing ≥ 8 GB), because this feature
changes the default slot kinds every wide and ultra project renders.
