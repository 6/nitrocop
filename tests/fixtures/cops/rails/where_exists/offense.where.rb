# nitrocop-config: EnforcedStyle: where

User.exists?(active: true)
     ^^^^^^^ Rails/WhereExists: Use `where(...).exists?` instead of `exists?(...)`.

Edition
  .scheduled
  .where("scheduled_publication <= ?", 5.seconds.from_now)
  .exists?(document_id: id)
   ^^^^^^^ Rails/WhereExists: Use `where(...).exists?` instead of `exists?(...)`.

seller
  .sales
  .not_subscription_or_original_purchase
  .exists?(email:)
   ^^^^^^^ Rails/WhereExists: Use `where(...).exists?` instead of `exists?(...)`.
