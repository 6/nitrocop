# nitrocop-expect: 3:6 Style/TrailingCommaInHashLiteral: Put a comma after the last item of a multiline hash.
hash = {
  a: 1,
  b: 2
}

single = {a: 1, b: 2}

# Corpus FN: nested single-line hash with trailing comma should still be rejected.
# nitrocop-expect: 13:57 Style/TrailingCommaInHashLiteral: Avoid comma after the last item of a hash, unless items are split onto multiple lines.
expect(subject.client).to receive(:post)
  .with(
    'business/getuserphonenumber',
    JSON.generate(code: 'xxxxxxxx'),
    hash_including(params: { access_token: 'access_token', }, base: Wechat::Api::WXA_BASE)
  )
