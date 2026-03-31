# The Constraint Principle

Real-time 3D graphics has a layered history. Early work was disciplined by constraint: wireframes, hard-flat shading, and budgets that made every polygon a decision. Later levels introduced expressive shading, physics-based materials, and scientific renderers capable of indistinguishable photorealism. The Constraint Principle is Level Six — not a revival of Level Two but a conscious choice to carry the lessons of every later level back into the formal discipline of early hardware.

The principle is structural: every slide runs on the same countable budgets. The vertex limit (60,000), the texture limit (4 MiB, 512×512 per texture), the tiny set of texture slots, and the shader contract are not technical ceilings but the congress of conditions that keep each slide obvious about its decisions. Reduction is not the goal. The goal is decision-making that remains visible because nothing can be added to hide it.

In practice that looks like these ongoing commitments:

- flat or quantised shading that lets geometry read clearly;
- intentional colour choices rather than texture-based illusion;
- slow, ambient motion (step finishes, bands, Bayer grain) that welcomes the peripheral eye;
- a refusal to chase the next shading trick or unlimited budget, because the trick is already to work in the bounded form.

The Constraint Principle is therefore both restraint and manifesto. It is not that VZGLYD cannot do more; it is that VZGLYD keeps choosing the conditions under which every triangle, every texture slot, every shader binding is a deliberate instrument. That is the form we ask of slide authors and agents alike.

*VZGLYD. Small worlds, well made.*
