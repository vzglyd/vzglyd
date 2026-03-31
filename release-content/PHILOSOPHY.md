# On Constraint

There was a specific moment when polygon budgets stopped being the binding
constraint in real-time 3D graphics, and it is worth being precise about when
that was: roughly 2005 to 2010, depending on the platform and the studio.
Before that moment, a character face might be built from three or four hundred
polygons. A complete game world — buildings, landscape, vegetation, distant
skyline — might come in under a hundred thousand triangles total. After that
moment, polygon counts entered the millions per frame, and the question
changed. It did not become a more interesting question. It largely stopped
being a question at all.

This matters because of what the constraint had been producing. When every
polygon costs something — when a tree cannot have more than thirty triangles
because there are forty trees in the scene and a budget to respect — artists
develop a different relationship to form. They learn to read geometry the way a
sculptor reads material: not filling space but carving it. They ask which
surfaces carry the meaning of the shape, which edges define the silhouette,
which faces catch light in the way the eye needs. Hard edges and flat faces are
not technical compromises in this mode of working. They are a visual
vocabulary. You can see the decisions in the work. The decisions are part of
what makes it interesting.

When the constraint relaxed, something happened to the decisions. The question
"which thirty polygons matter for this tree?" was not replaced by a better
question. It was replaced by no question at all, because the answer was simply:
all of them, and then more. More polygons, more texture resolution, more light
bounces, more geometry detail. The goal became photorealism — or rather, the
goal became the absence of the constraint itself, which is a different thing.
And the results are technically extraordinary and, a great deal of the time,
visually indistinguishable from each other. Everything renders at the same
resolution. Everything lights the same way. The discipline that came from
working in a tight corner quietly left the room.

This is not nostalgia. The argument is not that old work had charm and new work
lacks it. Charm is condescension in disguise. The argument is structural:
constraint forces decision-making, and decision-making produces work where the
choices are visible, and work where the choices are visible is interesting in a
way that work produced without constraints is not. This is why the most
visually distinctive 3D art ever made came out of the years when it was
genuinely difficult to make 3D art. Not despite the difficulty. Because of it.

Photorealism promises you will forget you are looking at a screen. That is its
entire operating ambition: to make the display surface disappear, to convince
the eye that it is looking through the glass rather than at it. Some VZGLYD
slides refuse that promise explicitly with visible dither or grain. Others do
it by simpler means: hard silhouettes, restrained colour, geometry that reads
at room scale. The point is not that every slide must share one treatment. The
point is that the slide should acknowledge the screen as a made surface and
make deliberate decisions within real limits.

The vertex limit is 60,000 per slide. A modern AAA character face is typically
around 15,000 polygons on its own. A complete VZGLYD world — the terrain slide,
for instance, which renders mountains, water, grass, rock, snow, and chimney
smoke from procedurally placed houses — builds all of that in 4,225 vertices.
The terrain mesh itself is a 65×65 grid. That is the whole landscape. It is not
impossibly small. The terrain is legible, the water catches light, the
snow-capped peaks read from a distance, the colour bands communicate altitude.
But you count. Every element has a cost. You learn what matters.

The 60,000 limit is not a technical ceiling imposed by hardware. VZGLYD runs on
hardware capable of far more. It is a chosen constraint, imposed deliberately,
for the same reason Oulipo writers chose formal constraints: not because the
constraint makes the work easier, but because the constraint makes the
decisions visible. Georges Perec wrote a 300-page novel without using the
letter E, not as a stunt, but because the constraint revealed things about
language that freedom had been obscuring. Constraint as method has a long and
serious history across creative disciplines. VZGLYD is applying it to real-time
3D graphics rendered to a television.

The comparison to these movements is not flattery. It is meant to locate the
approach in a tradition that understands what constraints are for. They are not
obstacles to overcome on the way to some unconstrained ideal. They are the
conditions under which certain kinds of interesting work become possible.
Freedom is often less useful than it sounds. When you can do anything, the
question of what to do becomes harder, not easier. When you can only do thirty
polygons for a tree, you learn which thirty matter, and the knowledge is real
and usable and visible in the result.

VZGLYD runs on a Raspberry Pi. It renders to a television. It lives in a room, in
the corner, while people are doing other things. The small worlds turning
inside it are not gallery pieces and they are not product demos. They are
objects in a domestic space making a quiet argument about what display is for,
at exactly the scale where that argument can be honest. Not a conference talk,
not a manifesto, not a studio pitch. A screen on a wall, a hard budget, a
bounded rendering contract, and 60,000 vertices to make a world from.

That is the right scale for this kind of work. Small enough to count. Large
enough to matter.
