use alloy::sol;

sol! {
    #[sol(rpc)]
    interface IClaimVerifier {
        enum ClaimState {
            None,
            Pending,
            Settled,
            Slashed,
        }

        struct Claim {
            address claimer;
            bytes32 workloadId;
            bytes32 artifactRoot;
            bytes32 resultRoot;
            uint64 createdAt;
            ClaimState state;
        }

        event ClaimSubmitted(
            uint256 indexed claimId,
            address indexed claimer,
            bytes32 indexed workloadId,
            bytes32 artifactRoot,
            bytes32 resultRoot
        );

        event ClaimChallenged(
            uint256 indexed claimId,
            address indexed challenger,
            bytes32 observedArtifactRoot,
            bytes32 observedResultRoot
        );

        event ClaimSettled(uint256 indexed claimId);

        event ClaimSlashed(uint256 indexed claimId);

        function submitClaim(
            bytes32 workloadId,
            bytes32 artifactRoot,
            bytes32 resultRoot
        ) external returns (uint256 claimId);

        function challengeClaim(
            uint256 claimId,
            bytes32 observedArtifactRoot,
            bytes32 observedResultRoot
        ) external;

        function settleClaim(uint256 claimId) external;

        function getClaim(uint256 claimId) external view returns (Claim memory);
    }
}
