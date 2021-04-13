// External deps
use num::ToPrimitive;
use zksync_crypto::franklin_crypto::{
    bellman::pairing::{
        bn256::{Bn256, Fr},
        ff::Field,
    },
    rescue::RescueEngine,
};
// Workspace deps
use zksync_crypto::{
    circuit::{
        account::CircuitAccountTree,
        utils::{append_be_fixed_width, le_bit_vector_into_field_element},
    },
    params::{
        account_tree_depth, ACCOUNT_ID_BIT_WIDTH, AMOUNT_EXPONENT_BIT_WIDTH,
        AMOUNT_MANTISSA_BIT_WIDTH, CHUNK_BIT_WIDTH, FEE_EXPONENT_BIT_WIDTH, FEE_MANTISSA_BIT_WIDTH,
        TOKEN_BIT_WIDTH, TX_TYPE_BIT_WIDTH,
    },
    primitives::FloatConversions,
};
use zksync_types::operations::SwapOp;
// Local deps
use crate::{
    operation::{Operation, OperationArguments, OperationBranch, OperationBranchWitness},
    utils::resize_grow_only,
    witness::{
        utils::{apply_leaf_operation, fr_from, get_audits, SigDataInput},
        Witness,
    },
};

#[derive(Debug)]
pub struct OrderData {
    pub account: u32,
    pub recipient: u32,
    pub amount: u128,
    pub price_sell: u128,
    pub price_buy: u128,
}

#[derive(Debug)]
pub struct SwapData {
    pub orders: (OrderData, OrderData),
    pub amounts: (u128, u128),
    pub tokens: (u32, u32),
    pub fee: u128,
    pub fee_token: u32,
    pub submitter: u32,
    pub valid_from: u64,
    pub valid_until: u64,
}

pub struct SwapWitness<E: RescueEngine> {
    pub accounts: (Vec<OperationBranch<E>>, Vec<OperationBranch<E>>),
    pub recipients: (Vec<OperationBranch<E>>, Vec<OperationBranch<E>>),
    pub submitter: OperationBranch<E>,
    pub args: OperationArguments<E>,
    pub roots: Vec<Option<E::Fr>>,
    pub tx_type: Option<E::Fr>,
    #[allow(clippy::type_complexity)]
    pub a_and_b: Vec<(Option<E::Fr>, Option<E::Fr>)>,
}

impl Witness for SwapWitness<Bn256> {
    type OperationType = SwapOp;
    type CalculateOpsInput = (SigDataInput, SigDataInput, SigDataInput);

    fn apply_tx(tree: &mut CircuitAccountTree, swap: &SwapOp) -> Self {
        let order_0 = OrderData {
            account: *swap.accounts.0 as u32,
            recipient: *swap.recipients.0 as u32,
            amount: swap.tx.orders.0.amount.to_u128().unwrap(),
            price_sell: swap.tx.orders.0.price.0.to_u128().unwrap(),
            price_buy: swap.tx.orders.0.price.1.to_u128().unwrap(),
        };

        let order_1 = OrderData {
            account: *swap.accounts.1 as u32,
            recipient: *swap.recipients.1 as u32,
            amount: swap.tx.orders.1.amount.to_u128().unwrap(),
            price_sell: swap.tx.orders.1.price.0.to_u128().unwrap(),
            price_buy: swap.tx.orders.1.price.1.to_u128().unwrap(),
        };

        let swap_data = SwapData {
            amounts: (
                swap.tx.amounts.0.to_u128().unwrap(),
                swap.tx.amounts.1.to_u128().unwrap(),
            ),
            tokens: (
                *swap.tx.orders.0.token_sell as u32,
                *swap.tx.orders.1.token_sell as u32,
            ),
            fee: swap.tx.fee.to_u128().unwrap(),
            fee_token: *swap.tx.fee_token as u32,
            orders: (order_0, order_1),
            submitter: *swap.submitter as u32,
            valid_from: swap.tx.valid_from(),
            valid_until: swap.tx.valid_until(),
        };

        Self::apply_data(tree, &swap_data)
    }

    fn get_pubdata(&self) -> Vec<bool> {
        // construct pubdata
        let mut pubdata_bits = vec![];
        append_be_fixed_width(&mut pubdata_bits, &self.tx_type.unwrap(), TX_TYPE_BIT_WIDTH);

        append_be_fixed_width(
            &mut pubdata_bits,
            &self.accounts.0[0].address.unwrap(),
            ACCOUNT_ID_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.recipients.0[0].address.unwrap(),
            ACCOUNT_ID_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.accounts.1[0].address.unwrap(),
            ACCOUNT_ID_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.recipients.1[0].address.unwrap(),
            ACCOUNT_ID_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.submitter.address.unwrap(),
            ACCOUNT_ID_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.accounts.0[0].token.unwrap(),
            TOKEN_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.accounts.1[0].token.unwrap(),
            TOKEN_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.submitter.token.unwrap(),
            TOKEN_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.args.amount_packed.unwrap(),
            AMOUNT_MANTISSA_BIT_WIDTH + AMOUNT_EXPONENT_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.args.second_amount_packed.unwrap(),
            AMOUNT_MANTISSA_BIT_WIDTH + AMOUNT_EXPONENT_BIT_WIDTH,
        );
        append_be_fixed_width(
            &mut pubdata_bits,
            &self.args.fee.unwrap(),
            FEE_MANTISSA_BIT_WIDTH + FEE_EXPONENT_BIT_WIDTH,
        );
        append_be_fixed_width(&mut pubdata_bits, &self.nonce_mask(), 8);

        resize_grow_only(&mut pubdata_bits, SwapOp::CHUNKS * CHUNK_BIT_WIDTH, false);
        pubdata_bits
    }

    fn get_offset_commitment_data(&self) -> Vec<bool> {
        vec![false; SwapOp::CHUNKS * 8]
    }

    fn calculate_operations(
        &self,
        input: (SigDataInput, SigDataInput, SigDataInput),
    ) -> Vec<Operation<Bn256>> {
        let pubdata_chunks: Vec<_> = self
            .get_pubdata()
            .chunks(CHUNK_BIT_WIDTH)
            .map(|x| le_bit_vector_into_field_element(&x.to_vec()))
            .collect();

        vec![
            Operation {
                new_root: self.roots[0],
                tx_type: self.tx_type,
                chunk: Some(fr_from(0)),
                pubdata_chunk: Some(pubdata_chunks[0]),
                first_sig_msg: Some(input.0.first_sig_msg),
                second_sig_msg: Some(input.0.second_sig_msg),
                third_sig_msg: Some(input.0.third_sig_msg),
                signature_data: input.0.signature.clone(),
                signer_pub_key_packed: input.0.signer_pub_key_packed.to_vec(),
                args: OperationArguments {
                    a: self.a_and_b[0].0,
                    b: self.a_and_b[0].1,
                    ..self.args.clone()
                },
                lhs: self.accounts.0[0].clone(),
                rhs: self.recipients.1[0].clone(),
            },
            Operation {
                new_root: self.roots[1],
                tx_type: self.tx_type,
                chunk: Some(fr_from(1)),
                pubdata_chunk: Some(pubdata_chunks[1]),
                first_sig_msg: Some(input.0.first_sig_msg),
                second_sig_msg: Some(input.0.second_sig_msg),
                third_sig_msg: Some(input.0.third_sig_msg),
                signature_data: input.0.signature.clone(),
                signer_pub_key_packed: input.0.signer_pub_key_packed.to_vec(),
                args: self.args.clone(),
                lhs: self.recipients.1[1].clone(),
                rhs: self.accounts.0[1].clone(),
            },
            Operation {
                new_root: self.roots[2],
                tx_type: self.tx_type,
                chunk: Some(fr_from(2)),
                pubdata_chunk: Some(pubdata_chunks[2]),
                first_sig_msg: Some(input.1.first_sig_msg),
                second_sig_msg: Some(input.1.second_sig_msg),
                third_sig_msg: Some(input.1.third_sig_msg),
                signature_data: input.1.signature.clone(),
                signer_pub_key_packed: input.1.signer_pub_key_packed.to_vec(),
                args: OperationArguments {
                    a: self.a_and_b[1].0,
                    b: self.a_and_b[1].1,
                    ..self.args.clone()
                },
                lhs: self.accounts.1[0].clone(),
                rhs: self.recipients.0[0].clone(),
            },
            Operation {
                new_root: self.roots[3],
                tx_type: self.tx_type,
                chunk: Some(fr_from(3)),
                pubdata_chunk: Some(pubdata_chunks[3]),
                first_sig_msg: Some(input.1.first_sig_msg),
                second_sig_msg: Some(input.1.second_sig_msg),
                third_sig_msg: Some(input.1.third_sig_msg),
                signature_data: input.1.signature.clone(),
                signer_pub_key_packed: input.1.signer_pub_key_packed.to_vec(),
                args: self.args.clone(),
                lhs: self.recipients.0[1].clone(),
                rhs: self.accounts.1[1].clone(),
            },
            Operation {
                new_root: self.roots[4],
                tx_type: self.tx_type,
                chunk: Some(fr_from(4)),
                pubdata_chunk: Some(pubdata_chunks[4]),
                first_sig_msg: Some(input.2.first_sig_msg),
                second_sig_msg: Some(input.2.second_sig_msg),
                third_sig_msg: Some(input.2.third_sig_msg),
                signature_data: input.2.signature.clone(),
                signer_pub_key_packed: input.2.signer_pub_key_packed.to_vec(),
                args: OperationArguments {
                    a: self.a_and_b[2].0,
                    b: self.a_and_b[2].1,
                    ..self.args.clone()
                },
                lhs: self.submitter.clone(),
                rhs: self.submitter.clone(),
            },
        ]
    }
}

impl SwapWitness<Bn256> {
    fn nonce_mask(&self) -> Fr {
        // a = 0 if orders.0.amount == 0 else 1
        // b = 0 if orders.1.amount == 0 else 1
        // nonce_mask = a | (b << 1)
        let mut nonce_mask = Fr::zero();
        nonce_mask.add_assign(&nonce_increment(&self.args.special_amounts[3]));
        nonce_mask.double();
        nonce_mask.add_assign(&nonce_increment(&self.args.special_amounts[0]));
        nonce_mask
    }

    fn apply_data(tree: &mut CircuitAccountTree, swap: &SwapData) -> Self {
        assert_eq!(tree.capacity(), 1 << account_tree_depth());
        let account_0_fe = fr_from(swap.orders.0.account);
        let account_1_fe = fr_from(swap.orders.1.account);
        let recipient_0_fe = fr_from(swap.orders.0.recipient);
        let recipient_1_fe = fr_from(swap.orders.1.recipient);
        let submitter_fe = fr_from(swap.submitter);
        let token_0_fe = fr_from(swap.tokens.0);
        let token_1_fe = fr_from(swap.tokens.1);
        let fee_token_fe = fr_from(swap.fee_token);
        let amount_0_fe = fr_from(swap.amounts.0);
        let amount_1_fe = fr_from(swap.amounts.1);
        let fee_fe = fr_from(swap.fee);

        let amount_0_bits = FloatConversions::to_float(
            swap.amounts.0,
            AMOUNT_EXPONENT_BIT_WIDTH,
            AMOUNT_MANTISSA_BIT_WIDTH,
            10,
        )
        .unwrap();

        let amount_1_bits = FloatConversions::to_float(
            swap.amounts.1,
            AMOUNT_EXPONENT_BIT_WIDTH,
            AMOUNT_MANTISSA_BIT_WIDTH,
            10,
        )
        .unwrap();

        let fee_bits = FloatConversions::to_float(
            swap.fee,
            FEE_EXPONENT_BIT_WIDTH,
            FEE_MANTISSA_BIT_WIDTH,
            10,
        )
        .unwrap();

        let amount_0_encoded: Fr = le_bit_vector_into_field_element(&amount_0_bits);
        let amount_1_encoded: Fr = le_bit_vector_into_field_element(&amount_1_bits);
        let fee_encoded: Fr = le_bit_vector_into_field_element(&fee_bits);
        let valid_from = swap.valid_from;
        let valid_until = swap.valid_until;

        let mut roots = vec![];
        let mut lhs_paths = vec![];
        let mut rhs_paths = vec![];
        let mut witnesses = vec![];

        let special_amounts: Vec<_> = vec![
            swap.orders.0.amount,
            swap.orders.0.price_sell,
            swap.orders.0.price_buy,
            swap.orders.1.amount,
            swap.orders.1.price_sell,
            swap.orders.1.price_buy,
        ]
        .into_iter()
        .map(|x| Some(fr_from(x)))
        .collect();

        lhs_paths.push(get_audits(tree, swap.orders.0.account, swap.tokens.0));
        rhs_paths.push(get_audits(tree, swap.orders.1.recipient, swap.tokens.0));

        witnesses.push(apply_leaf_operation(
            tree,
            swap.orders.0.account,
            swap.tokens.0,
            |acc| {
                acc.nonce.add_assign(&nonce_increment(&special_amounts[0]));
            },
            |bal| {
                bal.value.sub_assign(&amount_0_fe);
            },
        ));

        roots.push(tree.root_hash());
        lhs_paths.push(get_audits(tree, swap.orders.0.account, swap.tokens.0));
        rhs_paths.push(get_audits(tree, swap.orders.1.recipient, swap.tokens.0));

        witnesses.push(apply_leaf_operation(
            tree,
            swap.orders.1.recipient,
            swap.tokens.0,
            |_| {},
            |bal| bal.value.add_assign(&amount_0_fe),
        ));

        roots.push(tree.root_hash());
        lhs_paths.push(get_audits(tree, swap.orders.1.account, swap.tokens.1));
        rhs_paths.push(get_audits(tree, swap.orders.0.recipient, swap.tokens.1));

        witnesses.push(apply_leaf_operation(
            tree,
            swap.orders.1.account,
            swap.tokens.1,
            |acc| {
                acc.nonce.add_assign(&nonce_increment(&special_amounts[3]));
            },
            |bal| {
                bal.value.sub_assign(&amount_1_fe);
            },
        ));

        roots.push(tree.root_hash());
        lhs_paths.push(get_audits(tree, swap.orders.1.account, swap.tokens.1));
        rhs_paths.push(get_audits(tree, swap.orders.0.recipient, swap.tokens.1));

        witnesses.push(apply_leaf_operation(
            tree,
            swap.orders.0.recipient,
            swap.tokens.1,
            |_| {},
            |bal| bal.value.add_assign(&amount_1_fe),
        ));

        roots.push(tree.root_hash());
        lhs_paths.push(get_audits(tree, swap.submitter, swap.fee_token));

        witnesses.push(apply_leaf_operation(
            tree,
            swap.submitter,
            swap.fee_token,
            |acc| {
                acc.nonce.add_assign(&Fr::one());
            },
            |bal| bal.value.sub_assign(&fee_fe),
        ));

        roots.push(tree.root_hash());

        let a_and_b = vec![
            (witnesses[0].2, amount_0_fe),
            (witnesses[2].2, amount_1_fe),
            (witnesses[4].2, fee_fe),
        ];

        SwapWitness {
            accounts: (
                vec![
                    OperationBranch {
                        address: Some(account_0_fe),
                        token: Some(token_0_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[0].0.clone(),
                            balance_value: Some(witnesses[0].2),
                            account_path: lhs_paths[0].0.clone(),
                            balance_subtree_path: lhs_paths[0].1.clone(),
                        },
                    },
                    OperationBranch {
                        address: Some(account_0_fe),
                        token: Some(token_0_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[0].1.clone(),
                            balance_value: Some(witnesses[0].3),
                            account_path: lhs_paths[1].0.clone(),
                            balance_subtree_path: lhs_paths[1].1.clone(),
                        },
                    },
                ],
                vec![
                    OperationBranch {
                        address: Some(account_1_fe),
                        token: Some(token_1_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[2].0.clone(),
                            balance_value: Some(witnesses[2].2),
                            account_path: lhs_paths[2].0.clone(),
                            balance_subtree_path: lhs_paths[2].1.clone(),
                        },
                    },
                    OperationBranch {
                        address: Some(account_1_fe),
                        token: Some(token_1_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[2].1.clone(),
                            balance_value: Some(witnesses[2].3),
                            account_path: lhs_paths[3].0.clone(),
                            balance_subtree_path: lhs_paths[3].1.clone(),
                        },
                    },
                ],
            ),
            recipients: (
                vec![
                    OperationBranch {
                        address: Some(recipient_1_fe),
                        token: Some(token_1_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[3].0.clone(),
                            balance_value: Some(witnesses[3].2),
                            account_path: rhs_paths[2].0.clone(),
                            balance_subtree_path: rhs_paths[2].1.clone(),
                        },
                    },
                    OperationBranch {
                        address: Some(recipient_1_fe),
                        token: Some(token_1_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[3].0.clone(),
                            balance_value: Some(witnesses[3].2),
                            account_path: rhs_paths[3].0.clone(),
                            balance_subtree_path: rhs_paths[3].1.clone(),
                        },
                    },
                ],
                vec![
                    OperationBranch {
                        address: Some(recipient_0_fe),
                        token: Some(token_0_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[1].0.clone(),
                            balance_value: Some(witnesses[1].2),
                            account_path: rhs_paths[0].0.clone(),
                            balance_subtree_path: rhs_paths[0].1.clone(),
                        },
                    },
                    OperationBranch {
                        address: Some(recipient_0_fe),
                        token: Some(token_0_fe),
                        witness: OperationBranchWitness {
                            account_witness: witnesses[1].0.clone(),
                            balance_value: Some(witnesses[1].2),
                            account_path: rhs_paths[1].0.clone(),
                            balance_subtree_path: rhs_paths[1].1.clone(),
                        },
                    },
                ],
            ),
            submitter: OperationBranch {
                address: Some(submitter_fe),
                token: Some(fee_token_fe),
                witness: OperationBranchWitness {
                    account_witness: witnesses[4].0.clone(),
                    balance_value: Some(witnesses[4].2),
                    account_path: lhs_paths[4].0.clone(),
                    balance_subtree_path: lhs_paths[4].1.clone(),
                },
            },
            args: OperationArguments {
                amount_packed: Some(amount_0_encoded),
                second_amount_packed: Some(amount_1_encoded),
                full_amount: Some(amount_0_fe),
                // TODO: second full amount?
                // TODO: special_nonces?
                fee: Some(fee_encoded),
                valid_from: Some(fr_from(valid_from)),
                valid_until: Some(fr_from(valid_until)),
                special_accounts: vec![
                    Some(account_0_fe),
                    Some(account_1_fe),
                    Some(recipient_0_fe),
                    Some(recipient_1_fe),
                    Some(submitter_fe),
                ],
                special_tokens: vec![Some(token_0_fe), Some(token_1_fe), Some(fee_token_fe)],
                special_amounts,
                ..Default::default()
            },
            a_and_b: a_and_b
                .into_iter()
                .map(|(x, y)| (Some(x), Some(y)))
                .collect(),
            roots: roots.into_iter().map(Some).collect(),
            tx_type: Some(fr_from(SwapOp::OP_CODE)),
        }
    }
}

fn nonce_increment(amount: &Option<Fr>) -> Fr {
    if amount.unwrap().is_zero() {
        Fr::zero()
    } else {
        Fr::one()
    }
}
