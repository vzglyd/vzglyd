# VZGLYD

---

**The most interesting 3D art ever made had fewer triangles than a modern game's doorknob.**

---

There was a window — roughly twenty years ago, give or take — when three-dimensional graphics were new enough that nobody had figured out the rules yet. The hardware was slow. Memory was expensive. A single character's face might be built from three hundred polygons. An entire city from fifty thousand triangles. Artists had almost nothing to work with.

So they had to be clever.

They had to ask: what actually matters here? What can a bold, flat colour communicate that a photorealistic texture cannot? Where can a single hard edge carry more weight than a dozen soft ones? If you only have thirty polygons for a tree, which thirty matter?

The results — and this is the thing that people forget, or maybe never noticed — are often more visually interesting than work made today with effectively unlimited budgets and no constraints at all. Not despite the limitations. Because of them.

Constraint isn't a problem to solve. It's what forces interesting decisions.

---

Then budgets grew. Hardware got faster. The constraint relaxed, then disappeared. Studios poured resources into rendering photorealistic pores on faces and individual blades of grass on lawns, and somewhere in that pursuit of more, the discipline quietly left the room. Now everything renders at the same resolution. Everything lights the same way. Everything looks expensive and a lot of it looks the same.

A lot of people noticed something was off without being able to name it. The thing they're missing is the art that comes out of a tight corner.

---

**VZGLYD is a display engine for people who want that back.**

It runs on a Raspberry Pi. It renders to your television. It asks you to make something — a clock, a weather display, a word of the day, a piece of pure geometry that rotates on your wall — within a discipline. Small polygon counts. Intentional shading. No photorealism. No escaping the constraint by throwing hardware at it.

Each thing you make for VZGLYD is called a slide. A slide is a small program — a couple of hundred lines of Rust at most — that runs sixty times a second and describes a small world. The engine renders that world to your screen and leaves it there, all day, quietly doing its thing while you live your life around it.

It is, depending on how you want to think about it: ambient art, a functional display, a hobby project, or a small argument made visible on your wall about what makes graphics interesting.

---

## What it looks like

*[photographs here — the actual hardware on actual televisions in actual rooms]*

The screenshots don't do it justice. It needs to be seen on a screen in a room.

---

## Getting started

**You will need:** a Raspberry Pi 4, an HDMI cable, a television or monitor, and about twenty minutes.

```bash
curl -fsSL https://github.com/vzglyd/vzglyd/releases/latest/download/install.sh | sudo bash
```

That's the install. It sets everything up. Plug the Pi into the TV, run the command, reboot. VZGLYD starts on boot and runs the slides you have installed.

The default installation includes a clock, a weather display, and a few geometry pieces so you have something to look at while you figure out what you want to make next.

---

## Making your own slide

A slide is a Rust crate that compiles to WebAssembly. You describe geometry. The engine renders it. You don't touch the GPU directly — you just describe what you want, and VZGLYD works out how to draw it.

The constraint that makes this interesting: you're building for a Pi. That means low polygon counts are not just acceptable, they're correct. A well-made 400-triangle scene on a 55-inch television is a different kind of beautiful than a 4 million triangle scene. Different does not mean lesser.

**Scaffold a new slide:**

```bash
cargo install cargo-generate
cargo generate gh:vzglyd/slide-template --name my_slide
cd my_slide
bash build.sh
```

The generated project compiles and loads without modification. Start there, delete what you don't need, add what you do.

Full authoring guide: [docs/authoring-guide.md](docs/authoring-guide.md)

---

## Official slides

| Slide | What it does | Data source |
|-------|-------------|-------------|
| `clock` | Analogue and digital time | System clock, no network |
| `weather` | 3-day forecast | Bureau of Meteorology (AU) |
| `air_quality` | Pollen and air quality index | Public API |
| `afl` | AFL ladder and results | Squiggle |
| `calendar` | Upcoming events | Any ICS calendar URL |
| `lastfm` | Now playing / recent tracks | Last.fm |
| `on_this_day` | Historical events for today's date | Wikipedia |
| `word_of_day` | Word, definition, etymology | Dictionary API |
| `news` | Headlines | RSS, Reddit, Hacker News |
| `servers` | Uptime for a list of hosts | Your own servers |
| `quotes` | Rotating quote display | Compiled in — no network |
| `terrain` | 3D generative landscape | Coinbase BTC price drives terrain height |

More slides in the [registry](https://github.com/vzglyd/registry).

---

## The aesthetic

There isn't a strict rulebook. But there are things that feel right for VZGLYD and things that don't.

**Flat shading.** A hard edge between faces, each face its own colour. This is the look. It communicates geometry without pretending the geometry is something it isn't.

**Low polygon counts as a deliberate choice, not a budget.** If you can do it in a hundred triangles, do it in a hundred triangles. The right number of triangles is the fewest triangles that communicate the idea.

**Bold colour over photorealistic texture.** A single vivid green says more than a 4K grass texture. It says: this is grass, and I have decided it is this green, and I am not pretending otherwise.

**Things that move slowly.** VZGLYD lives in your room. It lives in your peripheral vision. It is not competing for your attention — it is ambient. Slow rotation. Gentle transitions. Give it permission to be quiet.

**Not trying to look like anything else.** Not trying to look like a phone widget, not trying to look like a desktop application, not trying to look like a 2024 AAA game. This is its own thing.

A fuller version of this thinking: [docs/aesthetic.md](docs/aesthetic.md)

---

## Contributing

The most valuable contribution is a slide that looks good and does something useful or interesting. Open a PR against the [registry](https://github.com/vzglyd/registry) once it's built and tagged.

For the engine itself: [CONTRIBUTING.md](CONTRIBUTING.md)

---

## Supporting the project

VZGLYD is made by one person with a television, a Raspberry Pi, and strong opinions about polygon budgets.

If it has been useful or interesting to you:

[![GitHub Sponsors](https://img.shields.io/badge/sponsor-♥-ea4aaa?style=flat-square)](https://github.com/sponsors/rodgerbenham)
[![Merch](https://img.shields.io/badge/merch-hat-black?style=flat-square)](https://bonfire.com/VRX-64-merch)

Gold sponsors get an embroidered VZGLYD hat, shipped to wherever you are in the world. It looks good. People ask about it.

---

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

---

*VZGLYD. Small worlds, well made.*
