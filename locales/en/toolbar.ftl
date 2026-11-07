# The drawing toolbar: tool names, buttons and their tooltips.
# A name is also the button's accessible name. $key: the key that does the same, as the
# app shows it ("P", "Ctrl+S").

## Tools

tool-pen = Pen
tool-pen-tooltip = Pen ({ $key }): pen pressure or mouse speed sets the width
tool-highlighter = Highlighter
tool-highlighter-tooltip = Highlighter ({ $key }): tints without covering text
tool-arrow = Arrow
tool-arrow-tooltip = Arrow ({ $key }): drag to draw
tool-rectangle = Rectangle
tool-rectangle-tooltip = Rectangle ({ $key }): hold { key-shift } for a square
tool-ellipse = Ellipse
tool-ellipse-tooltip = Ellipse ({ $key }): hold { key-shift } for a circle
tool-line = Line
tool-line-tooltip = Line ({ $key }): hold { key-shift } to snap to 45°
tool-text = Text
tool-text-tooltip = Text ({ $key }): click and type; { key-enter } finishes, { key-shift }+{ key-enter } adds a line
tool-stamp = Numbered stamps
tool-stamp-tooltip = Numbered stamps ({ $key }): each click places the next number
tool-blur = Blur
tool-blur-tooltip = Blur ({ $key }): drag over what to hide; on a frozen screen only
# The Blur tool picked in Live mode. $key: the freeze key.
tool-blur-needs-frozen = Blur works on a frozen screen only. Press { $key } to freeze it.
tool-eraser = Eraser
tool-eraser-tooltip = Eraser ({ $key }): erases whole strokes
tool-laser = Laser pointer
tool-laser-tooltip = Laser pointer ({ $key }): a glowing dot that leaves nothing behind

## Colour and width

toolbar-colour = Colour
toolbar-colour-tooltip = Colour (1 to 9)
toolbar-width = Width
toolbar-width-tooltip = Width: click to change, or hold a tool's key and scroll
toolbar-width-none-tooltip = Width: the eraser and the laser pointer have none
# $width: the width in pixels.
toolbar-width-value = { $width } px
toolbar-disappearing-ink = Disappearing ink
toolbar-disappearing-ink-tooltip = Disappearing ink: strokes fade a few seconds after you draw them

## A group's strip: Backdrop, Guides, Tool, Colour and Width each open a strip of their
## choices just below the toolbar.

# The strip, for screen readers. $group: the group's name ("Backdrop").
toolbar-strip = { $group } choices
toolbar-strip-close = Close the choices
toolbar-strip-close-tooltip = Close the choices; or click the group's button again

## Buttons

toolbar-save = Save
toolbar-save-tooltip = Save the screen ({ $key })
toolbar-save-region = Save a region
toolbar-save-region-tooltip = Save a region ({ $key })
toolbar-copy = Copy
toolbar-copy-tooltip = Copy the screen ({ $key })
toolbar-save-without-drawing = Save without the drawing
toolbar-save-without-drawing-tooltip = Save the screen without the drawing ({ $key })
toolbar-freeze = Freeze
toolbar-freeze-tooltip = Freeze the screen ({ $key })
toolbar-unfreeze = Unfreeze
toolbar-unfreeze-tooltip = Let the screen move again ({ $key })
# Greyed out on the whiteboard and the blackboard. $key: the freeze key.
toolbar-freeze-boards-tooltip = Freezing is for the live screen: choose Live, or press { $key } to freeze it
toolbar-undo = Undo
toolbar-undo-tooltip = Undo ({ $key })
# With Settings' "Right-click undoes" on. $key: the undo key.
toolbar-undo-right-click-tooltip = Undo ({ $key }, or right-click the drawing)
toolbar-redo = Redo
toolbar-redo-tooltip = Redo ({ $key })
toolbar-clear = Clear
toolbar-clear-tooltip = Clear the drawing ({ $key }); Undo brings it back
toolbar-close = Close
toolbar-close-tooltip = Close ({ $key }); the drawing goes
toolbar-collapse = Collapse
toolbar-collapse-tooltip = Collapse the toolbar
toolbar-expand = Expand
toolbar-expand-tooltip = Expand the toolbar
toolbar-more = More
toolbar-more-tooltip = More actions
# The toolbar window's name, for screen readers.
toolbar-name = Overmark toolbar
# The grip at the toolbar's left end.
toolbar-move = Move the toolbar
toolbar-move-tooltip = Drag to move the toolbar
toolbar-tool = Tool
toolbar-tool-tooltip = Tool: each has its own key
# $n: 1 to 9, the key that picks it.
toolbar-colour-n = Colour { $n }
toolbar-colour-n-tooltip = Colour { $n } ({ $n })

## Backdrop and guides. $next, $previous, $freeze: their keys ("W", "Shift+W", "F").

toolbar-backdrop = Backdrop
toolbar-backdrop-tooltip = Backdrop: { $next } for the next, { $previous } for the one before; { $freeze } freezes or unfreezes
toolbar-guides = Guides
toolbar-guides-tooltip = Guides: { $next } for the next, { $previous } for the one before

## Selecting a region

region-hint = Drag to select a region. { key-esc } cancels.
# $width, $height: the selection's size in pixels.
region-size = { $width } × { $height }

## Recording

toolbar-record = Record
toolbar-record-tooltip = Record a video of part of the screen, with your drawing: drag a box, then press Record on the bar above it
toolbar-record-busy-tooltip = A recording's box is open: use the bar on its edge
