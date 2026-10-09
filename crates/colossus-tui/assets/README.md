# Obscurity Labs terminal mark

`obscurity-mark.svg` retains the red mark paths from the Obscurity Labs flat
portrait logo. `obscurity-mark.braille` encodes its silhouette into 24 × 10
Unicode Braille cells (each cell represents 2 × 4 dots), using a 48 × 40 raster
and a 100/255 alpha threshold.

The TUI includes the encoded text directly. It needs no image protocol, runtime
asset lookup, or graphics dependency. The welcome renderer reduces the brightness
and saturation of the active theme's metadata color and shows it only when its
entire area is blank, including when startup warnings are present.
