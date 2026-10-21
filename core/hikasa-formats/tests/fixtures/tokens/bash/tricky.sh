echo a#b   # only this part is a comment   
count=$#; status=$?; pid=$$
msg='single quotes keep $HOME and \n as they are'
printf $'tab\there\n'
multi="first line
second line with \"quotes\""
echo ${#array[@]} ${var%%.*}
cat <<EOF
Don't panic
EOF
# a comment
echo done
