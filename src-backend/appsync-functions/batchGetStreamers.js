import { util } from '@aws-appsync/utils';
export function request(ctx) {
  const { broadcaster_ids } = ctx.arguments;
  return {
    operation: 'BatchGetItem',
    tables: {
      // ENV vars not available yet for AWS::Serverless::GraphQLApi
      ['notisr-streamers']: {
        keys: broadcaster_ids.map((id) => util.dynamodb.toMapValues({ broadcaster_id: id })),
        consistentRead: true,
      },
    },
  };
}

export function response(ctx) {
  if (ctx.error) {
    util.error(ctx.error.message, ctx.error.type);
  }
  return ctx.result.data['notisr-streamers'];
}
