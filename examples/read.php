<?php
$path = __DIR__ . '/../php_modules/deka.php';
$contents = file_get_contents($path);
echo $contents ? 'read:yes\n' : 'read:no\n';
