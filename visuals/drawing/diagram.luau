-- title: How a bike moves
-- category: drawing
-- description: A Mermaid flowchart; click a node for its explanation.
-- tags: diagram, flowchart, mermaid, explain
return function()
	return ui.kit.Diagram {
		mermaid = [[flowchart LR
  pedals[Pedals] --> crank[Crankset]
  crank --> chain[Chain]
  chain --> cassette[Cassette]
  cassette --> wheel[Rear wheel]
  shifter[Shifter] -.-> derailleur[Derailleur]
  derailleur -.-> chain]],
		details = {
			pedals = "Your legs push the pedals; each full turn moves the crank arms once around.",
			crank = "The crankset turns your push into rotation. A bigger chainring covers more ground per turn and is harder to push.",
			chain = "The chain carries the turning force from the chainring to the rear cog.",
			cassette = "The cassette is the stack of rear cogs. Smaller cogs give higher gears.",
			derailleur = "The derailleur pushes the chain sideways onto another cog when you shift.",
			shifter = "The shifter on the handlebar pulls or releases the cable that moves the derailleur.",
		},
	}
end
