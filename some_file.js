const foo = 'foo';
const someFunc = () => {
	const bar = 'syntax highlighting now';
	// we have comments now
	// and they carry over on newline
	return bar;
}

console.log(foo + '' + someFunc());
