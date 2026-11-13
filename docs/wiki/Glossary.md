# Glossary

Short meanings of the words that these pages use.

| Word | Meaning |
|---|---|
| **3MF** | A file format for 3D printing. It contains the surface of the model and can contain more data, such as colors and settings. |
| **Backend** | The part of OpenSCAD that calculates the shape. OpenSCAD has two: CGAL (older, slow) and Manifold (newer, fast). CoCAD needs Manifold. |
| **Bed** | The flat plate of a 3D printer on which the part is built. |
| **Bed chamfer** | A 45 degree chamfer of the edges of a part on the bed. CoCAD makes it in steps of whole layers. It hides the elephant foot. |
| **Bed island** | An area of the first layer that is not joined to the rest of the part in that layer, for example a thin foot. |
| **Bridge** | A flat ceiling over an empty space. The printer pulls strands across it from one edge to the other edge. |
| **Brim** | A thin layer of plastic around the bottom of a part, so that the part stays on the bed. CoCAD can add a brim of one layer to small bed islands. |
| **Build** | The OpenSCAD step: OpenSCAD calculates the shape of the model from the `.scad` file and the parameter values. Press `F5` to build. |
| **CAD** | Computer-aided design. A CAD program is a program in which you design parts. |
| **CLI** | Command line interface. You use the program with typed commands in a terminal, without a window. |
| **Customizer** | The function of OpenSCAD that shows the values of a model as sliders and lists. CoCAD reads the same data. |
| **Elephant foot** | The first layers of a print spread out a little, so the part is wider at the bed. |
| **Explode** | A view in which the parts of a model move away from each other, so that you see each part. |
| **Export** | To write the model to a file for a different program. |
| **Filament** | The plastic wire that a 3D printer melts to build a part. |
| **G-code** | The file with the movement commands for a 3D printer. A slicer writes it. CoCAD does not write G-code. |
| **Infill** | The pattern that fills the inside of a printed part. 100 % infill is a solid part. |
| **Joint** | The shape where two pieces of a split part go together: flat, pins or dovetail. |
| **Layer** | One thin, flat slice of a printed part. A printer builds the part one layer at a time. |
| **MCP** | Model Context Protocol. A standard that lets an AI assistant use the tools of a different program. |
| **MCP client** | A program with an AI assistant that can use MCP, for example Claude Desktop. |
| **Mesh** | The surface of a 3D model, made of many small triangles. |
| **Mouse ear** | A thin disk of one layer on a corner of a part. It holds the corner on the bed. You cut it off after the print. |
| **Model** | In CoCAD: a `.scad` file and the shape that it describes. |
| **Model definition** | The `.yml` file next to a model. It holds the title, the part names and the parameter sets. See [Model definition](Model-definition). |
| **Open edge** | A place where the surface of a mesh has a hole. A mesh with open edges is not a closed solid. |
| **OpenSCAD** | A free program in which you describe a 3D shape with text. CoCAD uses it to calculate the shape. |
| **Optimize** | The tab of CoCAD that prepares the parts for the printer: plates, orientation, print fixes, check and layers. |
| **Overhang** | A face that points down and is steeper than the overhang limit. It needs support. |
| **Parameter** | A value of the model that you can change, such as a width. |
| **Parameter set** | A named group of parameter values that you can save and load. The sets are in the model definition. |
| **Part** | One separate solid of a model, for example the lid of a box. |
| **Path tracer** | A method to make a picture that calculates how light moves in a scene. It gives soft shadows and real reflections. |
| **Plate** | One print job on the bed: the print parts and their positions. When the parts do not fit on one bed, CoCAD makes more plates. |
| **Print area** | The width, depth and height that your printer can print, in mm. |
| **Print fix** | A change of the shape that helps the printer: teardrop holes, hole size, bridges, bed chamfer, brim or mouse ears. |
| **Print part** | One row of the Plate card: a part, parts that overlap and join, or a piece of a split part. Parts with the same shape are one print part with a count. |
| **Reference part** | A part with the `%` mark in the model, for example a screw. CoCAD shows it in gray and never prints or exports it. |
| **Render** | To make a picture. The **Render** tab of CoCAD makes a photo of the part. |
| **Sacrificial layer** | One of the thin layers that close a hole above a bridge, so that the bridge is a full layer. You drill the hole open after the print. |
| **Sample** | One pass of the photo render over the image. More samples give a cleaner image. |
| **`.scad` file** | A model file of OpenSCAD. It is a text file. |
| **Section** | A cut through the model that shows the inside. |
| **Skin** | The solid layers at the top and the bottom of a printed part. |
| **Slice** | To cut a model into layers for a 3D printer. |
| **Slicer** | A program that slices a model and writes the file for the printer. PrusaSlicer and OrcaSlicer are slicers. |
| **Solid** | A closed shape with an inside and an outside. |
| **Split** | To cut a part that is too large for the print area into pieces, with joints. |
| **STEP** | A file format for CAD programs. It describes faces and solids, not triangles. |
| **STL** | A simple file format for 3D printing. It contains only the triangles of the surface. |
| **Support** | Extra plastic that a slicer adds under an overhang. You remove it after the print. |
| **Teardrop hole** | A round hole with a horizontal axis and a 45 degree point at the top. It prints without support. |
| **Wall** | A line of plastic that follows the outline of a layer. |
| **Wireframe** | A view that shows the edges of the triangles of the mesh. |
