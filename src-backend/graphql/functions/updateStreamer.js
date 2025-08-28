import { util } from '@aws-appsync/utils';

export function request(ctx) {
  const { broadcaster_id, ...rest } = ctx.args;

  const expressionNames = {};
  const expressionValues = {};
  const setParts = [];

  for (const [key, value] of Object.entries(rest)) {
    const name = `#${key}`;
    const val = `:${key}`;
    expressionNames[name] = key;

    if (typeof value !== 'undefined') {
      setParts.push(`${name} = ${val}`);
      expressionValues[val] = util.dynamodb.toDynamoDB(value);
    }
  }

  let expression = [];
  if (setParts.length > 0) {
    expression.push('SET ' + setParts.join(', '));
  }

  return {
    operation: 'UpdateItem',
    key: { broadcaster_id: util.dynamodb.toDynamoDB(broadcaster_id) },
    update: {
      expression: expression.join(' '),
      expressionNames,
      expressionValues,
    },
  };
}

export function response(ctx) {
  return ctx.result;
}
