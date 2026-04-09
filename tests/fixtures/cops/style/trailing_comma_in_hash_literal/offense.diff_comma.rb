# nitrocop-config: EnforcedStyle: diff_comma
# nitrocop-expect: 4:6 Style/TrailingCommaInHashLiteral: Put a comma after the last item of a multiline hash.
# nitrocop-expect: 14:25 Style/TrailingCommaInHashLiteral: Avoid comma after the last item of a hash, unless that item immediately precedes a newline.
hash = {
  a: 1,
  b: 2
}

Apipie::ParamDescription.new(
  method_description,
  name,
  Numeric,
  {
    in: "path",
    required: true,
    added_from_path: true, }
)

# Corpus FN: multiline hash without the final comma must still be flagged when
# the style override arrives via `EnforcedStyle`.
# nitrocop-expect: 25:46 Style/TrailingCommaInHashLiteral: Put a comma after the last item of a multiline hash.
auth_data = {
  provider: auth.provider,
  uid: auth.uid,
  auth: auth_hash.to_json,
  expires_at: expires_at,
  access_token: auth.credentials.token,
  access_token_secret: auth.credentials.secret
}
