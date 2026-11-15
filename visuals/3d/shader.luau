-- title: Plasma
-- category: 3d
-- description: A GLSL fragment shader in the Shadertoy style, with a speed input.
-- tags: shader, glsl, animation
return function()
	return ui.kit.Shader {
		height = 200,
		blend = "opaque",
		inputs = { speed = 1.5 },
		code = [[
void mainImage(out vec4 fragColor, in vec2 fragCoord) {
  vec2 uv = fragCoord / iResolution.xy;
  float t = iTime * speed;
  float v = sin(uv.x * 10.0 + t) + sin(uv.y * 8.0 - t) + sin((uv.x + uv.y) * 6.0 + t * 0.7);
  vec3 col = 0.5 + 0.5 * cos(vec3(0.0, 2.0, 4.0) + v);
  fragColor = vec4(col, 1.0);
}]],
	}
end
