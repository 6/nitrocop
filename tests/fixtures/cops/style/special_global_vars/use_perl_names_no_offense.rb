# nitrocop-config: EnforcedStyle: use_perl_names

alias $MATCH $&

alias $PREMATCH $`

alias $POSTMATCH $'

alias $LAST_PAREN_MATCH $+

/c(a)t/ =~ "cat"
$MATCH.should_not be_nil

/c(a)t/ =~ "cat"
$PREMATCH.should_not be_nil

/c(a)t/ =~ "cat"
$POSTMATCH.should_not be_nil

/c(a)t/ =~ "cat"
$LAST_PAREN_MATCH.should_not be_nil
