# nitrocop-config: EnforcedStyle: compact
g( f( x ))
g(( 3 + 5 ) * x )

warning( %(
  hi
) )

uri_parse( to_absolute( url, page.url )).scheme == "https"

case p
in Point( *, 1, *a )
  a
end

http.get( url, &method( :check_and_log ))
xml.input( name: replace_nulls( k ), value: replace_nulls( v ))
auditor.with_browser( options, &Submittable.prepare_callback( &block ))

outer(  ( x ))
outer( inner( x )  )
