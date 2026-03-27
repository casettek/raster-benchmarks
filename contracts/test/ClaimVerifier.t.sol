// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Test} from "forge-std/Test.sol";

import {ClaimVerifier} from "../src/ClaimVerifier.sol";
import {IClaimVerifier} from "../src/interfaces/IClaimVerifier.sol";

contract ClaimVerifierTest is Test {
    bytes32 internal constant INPUT_HASH =
        hex"0101010101010101010101010101010101010101010101010101010101010101";
    bytes32 internal constant TRACE_HASH =
        hex"0202020202020202020202020202020202020202020202020202020202020202";

    ClaimVerifier internal verifier;

    function setUp() external {
        verifier = new ClaimVerifier(120, 0.01 ether, 18 days);
        vm.deal(address(this), 10 ether);
    }

    function testRegisterManifestBlobsStoresFirstSeenMetadata() external {
        bytes32[] memory blobHashes = new bytes32[](1);
        blobHashes[0] = TRACE_HASH;

        vm.blobhashes(blobHashes);
        verifier.registerManifestBlobs();

        IClaimVerifier.BlobRegistration memory registration = verifier
            .getBlobRegistration(TRACE_HASH);
        assertEq(registration.blockNumber, uint64(block.number));
        assertEq(registration.timestamp, uint64(block.timestamp));
    }

    function testDuplicateRegistrationKeepsOriginalMetadata() external {
        bytes32[] memory blobHashes = new bytes32[](1);
        blobHashes[0] = TRACE_HASH;

        vm.blobhashes(blobHashes);
        verifier.registerManifestBlobs();

        IClaimVerifier.BlobRegistration memory original = verifier
            .getBlobRegistration(TRACE_HASH);

        vm.roll(block.number + 10);
        vm.warp(block.timestamp + 10);

        vm.blobhashes(blobHashes);
        verifier.registerManifestBlobs();

        IClaimVerifier.BlobRegistration memory duplicate = verifier
            .getBlobRegistration(TRACE_HASH);
        assertEq(duplicate.blockNumber, original.blockNumber);
        assertEq(duplicate.timestamp, original.timestamp);
    }

    function testSubmitClaimRequiresRegisteredFreshTraceHash() external {
        _registerBlob(TRACE_HASH);

        uint256 claimId = verifier.submitClaim{value: 0.01 ether}(
            bytes32(uint256(0x11)),
            bytes32(uint256(0x22)),
            1,
            1,
            bytes32(uint256(0x33)),
            bytes32(0),
            TRACE_HASH
        );

        assertEq(claimId, 1);
    }

    function testSubmitClaimRejectsUnregisteredTraceHash() external {
        vm.expectRevert("unregistered blob versioned hash");
        verifier.submitClaim{value: 0.01 ether}(
            bytes32(uint256(0x11)),
            bytes32(uint256(0x22)),
            1,
            1,
            bytes32(uint256(0x33)),
            bytes32(0),
            TRACE_HASH
        );
    }

    function testSubmitClaimRejectsStaleRegisteredHash() external {
        ClaimVerifier staleVerifier = new ClaimVerifier(120, 0.01 ether, 150);

        bytes32[] memory blobHashes = new bytes32[](1);
        blobHashes[0] = TRACE_HASH;
        vm.blobhashes(blobHashes);
        staleVerifier.registerManifestBlobs();

        vm.warp(block.timestamp + 31);

        vm.expectRevert("blob versioned hash too old");
        staleVerifier.submitClaim{value: 0.01 ether}(
            bytes32(uint256(0x11)),
            bytes32(uint256(0x22)),
            1,
            1,
            bytes32(uint256(0x33)),
            bytes32(0),
            TRACE_HASH
        );
    }

    function testSubmitClaimValidatesOptionalInputHashWhenPresent() external {
        _registerBlob(INPUT_HASH);
        _registerBlob(TRACE_HASH);

        uint256 claimId = verifier.submitClaim{value: 0.01 ether}(
            bytes32(uint256(0x11)),
            bytes32(uint256(0x22)),
            1,
            1,
            bytes32(uint256(0x33)),
            INPUT_HASH,
            TRACE_HASH
        );

        assertEq(claimId, 1);
    }

    function testRegisterManifestBlobsRejectsEmptyBlobContext() external {
        vm.expectRevert("missing blob hash");
        verifier.registerManifestBlobs();
    }

    function _registerBlob(bytes32 blobHash) internal {
        bytes32[] memory blobHashes = new bytes32[](1);
        blobHashes[0] = blobHash;
        vm.blobhashes(blobHashes);
        verifier.registerManifestBlobs();
    }
}
