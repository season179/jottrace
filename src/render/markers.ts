export function markerStart(name: string): string {
	return `<!-- jottrace:${name}:start v=1 -->`;
}

export function markerEnd(name: string): string {
	return `<!-- jottrace:${name}:end -->`;
}

export function renderRegion(name: string, content: string): string {
	return `${markerStart(name)}
${content}
${markerEnd(name)}`;
}
