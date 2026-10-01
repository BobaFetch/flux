#!/bin/sh
cat <<EOF
This is a here document.
    Its indent is kept.
if this were code
EOF

if true; then
    cat <<-END
	tab indented
	END
    echo inside
fi

cat <<'RAW'
$HOME is not expanded
  done
RAW
echo after

f() {
    cat <<MSG
hello {
MSG
}
