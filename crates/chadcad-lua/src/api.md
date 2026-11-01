# ChadCAD Lua API

Units are millimetres and degrees. Z is up. Any dimension may be a number or an
expression string over parameters, such as `"width / 2 + 3"`. Expressions support
`+ - * / % ^`, `pi`, `tau`, `sin cos tan asin acos atan sqrt abs floor ceil round min max`.

## Editing the document (console, AI)

Calls append to the timeline. A call taking a table accepts `name="..."`.
`op` is `"Join"` (default), `"Cut"` or `"Intersect"`. `axis` is `"X"`, `"Y"` or `"Z"` (default).

    cad.param("width", 40)               -- create or update; value may be an expression
    cad.box{size={"width", 30, 10}, origin={0, 0, 0}, op="Join"}
    cad.cylinder{radius=5, height=20, origin={0, 0, 0}, axis="Z", op="Cut"}
    cad.cone{base_radius=5, top_radius=2, height=10, origin=..., axis=..., op=...}
    cad.sphere{radius=5, center={0, 0, 0}, op=...}
    cad.torus{major=20, minor=3, center=..., axis=..., op=...}
    cad.extrude{sketch=SKETCH, distance=10, taper=0, op=...}
    cad.revolve{sketch=SKETCH, axis="Z", axis_origin={0, 0, 0}, angle=360, op=...}
    cad.fillet{edges=EDGES, radius=2}
    cad.chamfer{edges=EDGES, distance=1}
    cad.shell{open=FACES, thickness=2}   -- positive hollows inward, opening FACES
    cad.translate{by={10, 0, 0}}
    cad.rotate{angle=90, axis="Z", origin=...}
    cad.mirror{normal="X", origin=..., merge=true}
    cad.linear_pattern{count=4, spacing=20, direction={1, 0, 0}}
    cad.polar_pattern{count=6, angle=360, axis="Z", origin=...}
    cad.script{source="return box(1, 2, 3)", op=...}
    cad.remove(name)   cad.suppress(name, true)   cad.rollback(n or nil)
    cad.features()     -- list of feature names

SKETCH is `{plane="XY", offset=0, outer=PROFILE, holes={PROFILE, ...}}`; plane is
`"XY"`, `"XZ"` or `"YZ"`. PROFILE is one of:

    {Rect={size={40, 30}, center={0, 0}}}
    {Circle={radius=5, center={0, 0}}}
    {Polygon={points={{0, 0}, {10, 0}, {5, 8}}}}
    {RegularPolygon={radius=10, sides=6, center={0, 0}}}

EDGES is `"All"`, `"Curved"`, `{Parallel=2}` (0 = x, 1 = y, 2 = z), `{OnSide="Top"}` or
`{Nearest={x, y, z}}`. FACES is `{OnSide="Top"}` or `{Nearest={x, y, z}}`. Sides are
`Top Bottom Left Right Front Back`.

A fillet or chamfer applies to the body built so far, so add it after the cuts it
should follow. If an edit fails to build, the feature reports an error in the timeline.

## Script features

A `cad.script` source returns a solid. It sees `params.NAME` and, after the first
feature, `body`. It has no access to `cad`.

    box(x, y, z)   cylinder(r, h)   cone(r1, r2, h)   sphere(r)   torus(R, r)
    prism({{x, y}, ...}, h)
    a + b  a - b  a * b     -- or a:fuse(b), a:cut(b), a:common(b)
    s:translate(x, y, z)  s:rotate("z", deg)  s:mirror("x")
    s:fillet(r, sel)  s:chamfer(d, sel)  s:volume()

`sel` is `"all"` (default), `"curved"`, `"x" "y" "z"` (parallel to that axis) or a side:
`"top" "bottom" "left" "right" "front" "back"`.

The sandbox provides `math`, `string`, `table` and `utf8`, with a 30 second limit.
There is no file, OS, network or module access.
