-- A parametric mounting bracket. Build it with:
--   cargo run -p chadcad-cli -- lua examples/bracket.lua examples/bracket.chad

cad.param("width", 60)
cad.param("depth", 40)
cad.param("thickness", 6)
cad.param("height", 35)
cad.param("hole", 3.5)
cad.param("inset", "thickness + hole * 2")
cad.param("gusset", "height * 0.6")

cad.box{name="Base", size={"width", "depth", "thickness"}}
cad.box{name="Upright", size={"width", "thickness", "height"}}

cad.extrude{
    name="Gusset",
    sketch={
        plane="YZ",
        offset="width / 2 - thickness / 2",
        outer={Polygon={points={
            {"thickness", "thickness"},
            {"thickness + gusset", "thickness"},
            {"thickness", "thickness + gusset"},
        }}},
    },
    distance="thickness",
}

cad.cylinder{name="BoreLeft", origin={"inset", "depth - inset", -1},
    radius="hole", height="thickness + 2", op="Cut"}
cad.cylinder{name="BoreRight", origin={"width - inset", "depth - inset", -1},
    radius="hole", height="thickness + 2", op="Cut"}

cad.cylinder{name="SlotLeft", origin={"inset", -1, "height - inset"}, axis="Y",
    radius="hole", height="thickness + 2", op="Cut"}
cad.cylinder{name="SlotRight", origin={"width - inset", -1, "height - inset"}, axis="Y",
    radius="hole", height="thickness + 2", op="Cut"}

cad.fillet{name="Rounded", edges={Parallel=2}, radius=3}
