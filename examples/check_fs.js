const { op_php_file_exists } = Deno.core.ops;
const path = '/Users/samifouad/Projects/deka/deka/php_modules/deka.php';
console.log('exists', op_php_file_exists(path));
