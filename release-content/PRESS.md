# VZGLYD - Press Information

---

## Project description

VZGLYD is an open-source display engine for Raspberry Pi that renders 3D scenes
and animated data displays to a standard HDMI television. It runs on a
Raspberry Pi 4, uses wgpu for GPU-accelerated rendering, and executes slides —
individual visual programs — as WebAssembly modules compiled from Rust and
dropped into a watched directory as `.vzglyd` files. The aesthetics are
deliberate, but not fixed to one house style: low polygon counts, small texture
budgets, bounded shaders, and scenes built to read clearly at room scale. The
central argument of the project is that the discipline produced by hardware
constraint was not a limitation waiting to be overcome. It was a method. VZGLYD
practices that method on contemporary hardware, by choice, without requiring
every slide to share one lighting formula or one grain treatment.

---

## The facts

**Hardware and installation.** VZGLYD runs on a Raspberry Pi 4 connected to any
HDMI television or monitor. Installation is a single bash command and takes
approximately twenty minutes. No display server, no browser, no app framework —
a native rendering runtime writing directly to the screen.

**The engine.** Written in Rust. GPU rendering via wgpu. Slide execution via
wasmtime. Slides are WebAssembly modules with a stable ABI boundary between the
engine and the slide program. The engine owns rendering, frame scheduling,
transitions, asset management, and data provider integration. The slide owns
what it displays and how it behaves over time.

**The constraints.** 60,000 vertices per slide maximum. 4 megabytes total
texture budget. 512×512 maximum texture resolution. A small fixed shader
interface with a handful of texture slots. A portable WebAssembly slide ABI. No
unrestricted material system. Slides choose their own shading language within
that envelope. These are chosen constraints, not hardware ceilings.

**The slides.** Official slides available at launch include: clock (an analogue
clock with no authored geometry — bezel, hands, face, and glass dome
constructed entirely at runtime from primitive forms), golf (a cel-shaded
isometric golf course in impossible saturated colour), courtyard (a
Blender-authored contemplative 3D space), and weather (a Bureau of Meteorology
data display). Approximately twenty additional slides cover AFL scores, news,
music playback, calendar, air quality, server status, personal reminders,
language, and more. The data slides share a design language: very dark
backgrounds, cyan accent text, clean typographic hierarchy.

**Authoring.** The slide specification (`vzglyd-slide`) and sidecar networking
kit (`vzglyd-sidecar`) are published on crates.io. Anyone with Rust installed can
build a slide.

**License.** MIT or Apache 2.0. Open source.

---

## What it looks like

VZGLYD lives in a room. That is not a metaphor — the intended installation is a
television mounted or placed somewhere you already spend time, running quietly
while you do other things. Some slides use visible dither or grain. Others use
cleaner surfaces. Across them, the geometry reads differently on a 55-inch
screen than it does in a screenshot. The hard edges are large. The colour
decisions are large. The golf course grass — saturated past anything a real
fairway achieves — fills the room at scale. The terrain's snow-capped peaks
hold still in that particular way that only computed things hold still. The
clock's second hand advances in silence. None of it demands attention. All of
it rewards a glance. The display is not a window and it is not a notification.
It is an object in a domestic space with its own character — small worlds
turning, unhurried, in the corner.

*[Photographs of VZGLYD running on real televisions in real rooms would appear here.]*

---

## Why now

The photorealism race in AAA games reached a point, sometime in the last
decade, where the pursuit of more — more polygons, more accurate light
simulation, more surface detail — stopped being a means to an end and became
the end itself. The results are technically extraordinary. They are also, a
great deal of the time, visually indistinguishable from each other. The
discipline that came from working inside tight constraints quietly left the
room when the constraints relaxed. The indie scene recovered significant ground
with pixel art and hand-drawn work — those movements understood that constraint
produces decisions, and decisions produce character. Real-time 3D with
intentional constraint remains, by comparison, underexplored. VZGLYD is a small
argument in that space. It is not a studio project. It is one person, a
television, and a Raspberry Pi, making the case that the visual language of
constrained real-time 3D is worth practising deliberately — not as nostalgia,
but as method.

---

## Who made it

VZGLYD is a personal project built by a single developer with strong opinions
about what makes 3D graphics interesting. It is open source because the right
tools should be shared, and because the slides are more interesting when other
people are making them. The engine specification, the slide ABI, and the
sidecar networking kit are all published for anyone to build against.

---

## Angles and framings

**The hardware angle.** A Raspberry Pi 4 is roughly $80 of hardware. VZGLYD turns
it into an art machine that lives in a room. The constraint of the hardware and
the chosen formal limits reinforce each other: both force attention onto what
is worth rendering and what can be left out.

**The aesthetic movement angle.** VZGLYD belongs to a broader recovery of
constraint-based visual work: the lo-fi music movement, the pixel art
renaissance, the return of physical media aesthetics. These movements share an
understanding that constraint is not an obstacle but a condition under which
certain kinds of interesting work become possible. The demoscene has understood
this for thirty years and produces entire animated worlds in 4 kilobytes. VZGLYD
makes constraint-based real-time 3D accessible and domestic.

**The ambient information design angle.** The data slides — weather, calendar,
news, music, server status — are a specific argument about what information
should look like in a living space. Not a dashboard. Not a notification feed.
Not a Bloomberg terminal. Something quiet, legible from across the room, that
belongs in the space rather than competing with it. The dark palette keeps the
display from becoming a light source. The hierarchy lets you take in what
matters at a glance, without stopping what you are doing.

**The open source angle.** The engine is open. The slide specification and
sidecar networking kit are published on crates.io. The `.vzglyd` format is
documented. Anyone with Rust installed can build a slide, author geometry,
write shaders within the WGSL contract, and deploy to a running VZGLYD instance
by dropping a file into a directory.

**The nostalgia-but-not-nostalgia angle.** This is the most important angle to
get right. VZGLYD is not retro for retro's sake. It is not about feeling
nostalgic for Nintendo 64. The argument is structural, not sentimental:
hardware constraint forced decision-making, and decision-making produced work
where the choices are visible, and work where the choices are visible is
interesting in a way that work produced without constraint is not. VZGLYD imposes
those constraints deliberately, in 2026, on hardware capable of far more,
because the constraint is the point, not the limitation.

---

## Contact and links

- **GitHub:** [placeholder — repository link]
- **Sponsor:** [placeholder — sponsor link]
- **Slide registry:** [placeholder — registry link]

For the full argument behind the aesthetic, see `PHILOSOPHY.md` in the repository.

---

VZGLYD. Small worlds, well made.
