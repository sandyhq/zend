-- title: Sign-in handshake
-- category: drawing
-- description: A Mermaid sequence diagram of one request.
-- tags: diagram, sequence, mermaid, request
return function()
	return ui.kit.Diagram {
		mermaid = [[sequenceDiagram
  participant B as Browser
  participant A as API
  participant D as Database
  B->>A: POST /login
  A->>D: find user
  D-->>A: user row
  Note over A,D: password checked here
  A-->>B: session cookie]],
	}
end
