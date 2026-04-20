# nitrocop-config: EnforcedStyle: compact
g( f( x ))
g(( 3 + 5 ) * x )
wrap(  ( value ))
wrap( inner( value )  )

warning( %(
  hi
) )

uri_parse( to_absolute( url, page.url )).scheme == "https"
wrap( outer: inner(
  value
))

def initialize(
  fetch_authenticator_secrets: Authentication::Util::FetchAuthenticatorSecrets.new(
    optional_variable_names: %w[optional-signed-headers]
  ))
end

handler = ->( callback = build_callback( value )) { callback }

case p
in Point( *, 1, *a )
  a
end
