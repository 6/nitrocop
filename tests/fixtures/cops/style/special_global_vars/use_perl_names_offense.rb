# nitrocop-config: EnforcedStyle: use_perl_names

puts $PID
     ^^^^ Style/SpecialGlobalVars: Prefer `$$` over `$PID`.

puts $FS
     ^^^ Style/SpecialGlobalVars: Prefer `$;` over `$FS`.

puts $OFS
     ^^^^ Style/SpecialGlobalVars: Prefer `$,` over `$OFS`.

puts $RS
     ^^^ Style/SpecialGlobalVars: Prefer `$/` over `$RS`.

puts $ORS
     ^^^^ Style/SpecialGlobalVars: Prefer `$\` over `$ORS`.

puts $NR
     ^^^ Style/SpecialGlobalVars: Prefer `$.` over `$NR`.
