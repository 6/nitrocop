# nitrocop-config: EnforcedStyle: aggressive

Post.where(user_id: users.pluck(:id).map(&:to_i))
Post.where(user_id: User.pluck(:id).map(&:to_i))
Post.pluck(:id).where(id: 1..10)
