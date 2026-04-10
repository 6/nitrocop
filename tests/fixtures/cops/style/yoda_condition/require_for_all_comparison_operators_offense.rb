# nitrocop-config: EnforcedStyle: require_for_all_comparison_operators

assert_argerr { (2**500).<(1,2) }
                ^^^^^^^^^^^^^^^ Style/YodaCondition: Prefer Yoda conditions.
