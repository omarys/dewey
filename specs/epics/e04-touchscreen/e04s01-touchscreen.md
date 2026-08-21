# e04s01: Tap, double-tap, action bar, and portrait layout verification

## §1 Business narrative
As a tablet user (PineTab 2), I need to navigate dewey with touch — tap to select, double-tap to open, tappable buttons for actions — in portrait orientation where panes stack vertically.

## §5 Main flow
1. Tap on a row -> selects it and focuses that pane
2. Double-tap on same item -> opens it (series->chapters, chapter->Continuum)
3. Scroll wheel over a pane -> moves selection
4. Tap action bar button -> executes action
5. Portrait (height > width) -> panes stack vertically

## §6 Constraints
- Requires touch-to-mouse terminal (foot recommended; kitty scroll-only)
- crossterm delivers 0-based mouse coordinates
- Action bar buttons take precedence over list taps
- Delete button uses same double-press confirm as keyboard x

## §17 Gherkin
```gherkin
Scenario: Tap selects series
  Given the series list is displayed
  When I tap on "Solo Leveling" row
  Then "Solo Leveling" is selected and series pane is active

Scenario: Double-tap opens chapter
  Given a chapter is selected
  When I double-tap on it
  Then Continuum spawns with that chapter

Scenario: Portrait layout stacks panes
  Given a terminal with height > width
  When dewey renders
  Then series, chapters, and details panes are stacked vertically
```

## §18 Out of scope
- Drag-to-scroll (finger-drag scrolling)
- Multi-touch gestures (pinch-zoom)
- On-screen keyboard integration
