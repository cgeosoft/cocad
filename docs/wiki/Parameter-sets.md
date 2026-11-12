# Parameter sets

A parameter set is a named group of values for one model. For example, a box model can have the sets "small", "big" and "for the workshop". You select a set and all sliders move to the saved values.

![The parameter sets](https://raw.githubusercontent.com/cgeosoft/cocad/main/assets/tour/3-sets.jpg)

## Where to find them

The top of the **Parameters** panel has two items:

- A **card** with the name of the sets file. All sets of a model are in one file.
- A **list** of the sets, below the card.

## Usual tasks

| Task | How |
|---|---|
| Load a set | Select it in the list. |
| Go back to the values of the model file | Select **Model defaults** in the list. |
| Save your values as a new set | Click the gear button, then **New set…**. Type a name. |
| Save your changes to the selected set | Click the disk button. |
| Delete the selected set | Click the gear button, then **Delete**. Confirm in the window that opens. |
| Use sets from a different file | Click **Open** and select a `.json` file. |
| Start a new sets file | Click the gear button, then **New file…**. Select a place and a name. |

The disk button is orange while the set has changes that are not saved.

If no set is selected, the disk button asks for a name and makes a new set.

## What CoCAD remembers

- CoCAD remembers the sets file of each model. If you did not select one, it uses a file with the name of the model next to the model: `box.json` for `box.scad`.
- When you open a model, CoCAD loads the set that you used last for that model. If there is no record, it loads the first set of the file.
- If **Model defaults** was your last selection, the values stay at the defaults of the model.
- A new sets file does not exist on disk until your first save.

## Auto save

With auto save, CoCAD writes your changes to the selected set a moment after your last change (0.6 seconds).

1. Open **File > Settings**.
2. Select the **Parameter sets** page.
3. Turn on **Auto save**.
4. Click **Save**.

Auto save is off by default. With **Model defaults** selected, it saves nothing.

## When the sets file changes on disk

If another program changes the sets file, the list of sets updates. The active set gets the new values, unless you have changes in it that are not saved.

---

**For technical readers.** The file uses the format of the OpenSCAD Customizer, so OpenSCAD reads it too: `openscad -p sets.json -P big`. The [command line](Command-line) of CoCAD uses the same sets with `-P` and `-p`.
