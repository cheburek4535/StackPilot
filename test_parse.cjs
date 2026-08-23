const { parse } = require('svelte/compiler');
const ast = parse('<p class="subtitle" title="Подзаголовок">Привет, {name}!</p>');
console.log(JSON.stringify(ast.html, null, 2));
