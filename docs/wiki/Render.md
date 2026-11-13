# Render

The **Render** tab makes a picture of the part that looks like a studio photo, with soft light and shadows. You can use it for a shop page, a README or a message to a friend.

![The Render tab](https://raw.githubusercontent.com/cgeosoft/cocad/main/assets/tour/7-photo.jpg)

## Make a photo

1. Click the **Render** tab on the right, or press `R`. Press `R` again to go back to the **Model** tab.
2. Turn and move the view until you like it. The view dims the area outside the image frame, so you see what the photo will show.
3. Let the view rest. The render starts. The image gets cleaner with time. The **Progress** card shows the number of samples.
4. Click **Save PNG**. The dialog opens in the folder of your last export.

When you move the view or change a setting, the render starts again.

You can save at any time. **Save PNG** writes the image at its full size, with the quality that it has at that moment.

## Settings

### Material

- **Color**: 8 color swatches, or a color picker for your own color.
- **Finish**: **Matte**, **Satin** or **Gloss** plastic.
- **Print layer lines**: adds fine lines on the sides, so that the part looks 3D printed.

When [Part colors](Parts#part-colors) is on, each part has its own color in place of the material color. The finish and the layer lines stay.

### Scene

- **Studio**: a **Dark** or a **Light** background.
- **Exposure**: the brightness of the image.

### Quality

- **Samples**: 16 to 2048. More samples give a cleaner image and take more time.
- **Resolution**: the size of the image in pixels. The presets are 720p, 1080p, 1440p, 4K, site preview 4:3 (1600 × 1200), square, portrait 4:5 and story 9:16. You can also type the width and the height.
- **Swap** changes between landscape and portrait.

The cards can close and open, as in the other tabs. The **Export** card is at the bottom. See [Export](Export).

## Parts in the photo

The photo shows the model as built, with the [part colors](Parts#part-colors), the [explode](Parts#explode) and the [section](Parts#section) of the 3D view. It does not show the selection, the focus or the [reference parts](Parts#reference-parts).

The photo shows the model, not the print plate of the [Optimize](Optimize) tab.

---

**For technical readers.** The render is a path tracer that uses all CPU cores. One sample is one pass over all pixels of the image. `cocad photo` makes a photo in a terminal. See [Command line](Command-line).
