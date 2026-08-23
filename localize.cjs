const fs = require('fs');
const path = require('path');
const { parse } = require('svelte/compiler');

// We will parse the file, find all Text nodes and Attribute values with Cyrillic, 
// and generate a patch for the file. 
console.log(parse);
